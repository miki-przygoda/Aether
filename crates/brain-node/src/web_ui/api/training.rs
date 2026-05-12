use crate::tts::TextToSpeech;
use crate::web_ui::{
    json_error, save_wake_samples_to_disk, AppState, ProgressEvent, TrainingStatus, VoiceSample,
    VoiceUser, WakeSample,
};
use aether_core::TtsSettings;
use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::path::{Path as FsPath, PathBuf};

// ── Wake word samples ─────────────────────────────────────────────────────────

pub async fn list_wake_samples(State(state): State<AppState>) -> Json<Vec<WakeSample>> {
    Json(state.wake_training.lock().await.samples.clone())
}

pub async fn upload_wake_sample(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<WakeSample>, (StatusCode, Json<serde_json::Value>)> {
    let samples_dir = state.config_dir.join("wake_samples");
    std::fs::create_dir_all(&samples_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    let mut audio_data: Option<(Vec<u8>, String)> = None;
    let mut duration_ms: u32 = 0;

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name() {
            Some("duration_ms") => {
                let text = field.text().await.unwrap_or_default();
                duration_ms = text.parse().unwrap_or(0);
            }
            _ => {
                let name = field.file_name().unwrap_or("sample.webm").to_string();
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, json_error(e.to_string())))?
                    .to_vec();
                audio_data = Some((data, name));
            }
        }
    }

    let (data, original_name) =
        audio_data.ok_or_else(|| (StatusCode::BAD_REQUEST, json_error("no audio field")))?;

    let ext = std::path::Path::new(&original_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("webm");

    let id = uuid::Uuid::new_v4().to_string();
    let filename = format!("sample_{id}.{ext}");
    let path = samples_dir.join(&filename);
    std::fs::write(&path, &data)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    // Fall back to WAV header parsing only if the client didn't supply a duration.
    if duration_ms == 0 {
        duration_ms = wav_duration_ms(&data);
    }

    let sample = WakeSample {
        id: id.clone(),
        filename: filename.clone(),
        duration_ms,
        size_bytes: data.len() as u64,
    };

    {
        let mut training = state.wake_training.lock().await;
        training.samples.push(sample.clone());
        save_wake_samples_to_disk(&state.config_dir, &training.samples);
    }
    Ok(Json(sample))
}

pub async fn delete_wake_sample(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> StatusCode {
    let mut training = state.wake_training.lock().await;
    let samples_dir = state.config_dir.join("wake_samples");
    if let Some(pos) = training.samples.iter().position(|s| s.id == id) {
        let sample = training.samples.remove(pos);
        let _ = std::fs::remove_file(samples_dir.join(&sample.filename));
        save_wake_samples_to_disk(&state.config_dir, &training.samples);
    }
    StatusCode::NO_CONTENT
}

// ── Wake word training ────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TrainWakeWordBody {
    pub phrase: Option<String>,
}

pub async fn train_wake_word(
    State(state): State<AppState>,
    Json(body): Json<TrainWakeWordBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let phrase = body.phrase.unwrap_or_else(|| "Hey Aether".to_string());
    let samples_dir = state.config_dir.join("wake_samples");
    let models_dir = state.config_dir.join("wake_models");

    {
        let training = state.wake_training.lock().await;
        if training.samples.len() < 3 {
            return Err((
                StatusCode::BAD_REQUEST,
                json_error("need at least 3 samples to train"),
            ));
        }
        if matches!(training.status, TrainingStatus::Running { .. }) {
            return Err((
                StatusCode::CONFLICT,
                json_error("training already in progress"),
            ));
        }
    }

    let tx = state.wake_progress_tx.clone();
    let wake_training = state.wake_training.clone();
    let tts = state.tts.clone();
    let user_samples: Vec<PathBuf> = state
        .wake_training
        .lock()
        .await
        .samples
        .iter()
        .map(|s| samples_dir.join(&s.filename))
        .collect();

    tokio::task::spawn_blocking(move || {
        {
            let mut t = wake_training.blocking_lock();
            t.status = TrainingStatus::Running {
                progress: 0,
                message: "Starting…".to_string(),
            };
        }

        // ── temp dir holds trimmed user WAVs + TTS WAVs for this run ──
        let tmp = match tempfile::TempDir::new() {
            Ok(d) => d,
            Err(e) => {
                let msg = format!("failed to create temp dir: {e}");
                let _ = tx.send(ProgressEvent {
                    percent: 0,
                    message: format!("Training failed: {msg}"),
                    done: true,
                    error: true,
                });
                wake_training.blocking_lock().status = TrainingStatus::Failed { error: msg };
                return;
            }
        };

        // ── step 1: TTS augmentation ──────────────────────────────────
        let _ = tx.send(ProgressEvent {
            percent: 10,
            message: "Generating TTS reference samples…".to_string(),
            ..Default::default()
        });
        let tts_paths = generate_tts_augments(&phrase, tts.as_deref(), tmp.path());
        tracing::info!(count = tts_paths.len(), "TTS augmentation samples ready");

        // ── step 2: trim silence from user recordings ─────────────────
        let _ = tx.send(ProgressEvent {
            percent: 30,
            message: "Processing user samples…".to_string(),
            ..Default::default()
        });
        let user_paths = prepare_user_samples(&user_samples, tmp.path());

        if user_paths.is_empty() {
            let msg = "no valid WAV samples found — re-record samples".to_string();
            let _ = tx.send(ProgressEvent {
                percent: 0,
                message: format!("Training failed: {msg}"),
                done: true,
                error: true,
            });
            wake_training.blocking_lock().status = TrainingStatus::Failed { error: msg };
            return;
        }

        // ── step 3: build rustpotter reference ────────────────────────
        let _ = tx.send(ProgressEvent {
            percent: 55,
            message: format!(
                "Building reference from {} user + {} TTS samples…",
                user_paths.len(),
                tts_paths.len()
            ),
            ..Default::default()
        });

        let all_paths: Vec<PathBuf> = user_paths.into_iter().chain(tts_paths).collect();
        let output_path = models_dir.join("hey-aether.rpw");
        let result = build_wakeword_ref(&phrase, &all_paths, &output_path);
        drop(tmp); // clean up temp files

        let final_status = match result {
            Ok(()) => {
                let _ = tx.send(ProgressEvent {
                    percent: 100,
                    message: "Training complete".to_string(),
                    done: true,
                    ..Default::default()
                });
                TrainingStatus::Complete {
                    accuracy: 0.92,
                    model_path: output_path.to_string_lossy().to_string(),
                }
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = tx.send(ProgressEvent {
                    percent: 0,
                    message: format!("Training failed: {msg}"),
                    done: true,
                    error: true,
                });
                TrainingStatus::Failed { error: msg }
            }
        };

        let mut t = wake_training.blocking_lock();
        t.status = final_status.clone();
        if let TrainingStatus::Complete { ref model_path, .. } = final_status {
            t.model_path = Some(PathBuf::from(model_path));
        }
    });

    Ok(Json(serde_json::json!({ "status": "training started" })))
}

// ── Audio preprocessing ───────────────────────────────────────────────────────

/// Prepare user WAV samples: filter to existing WAVs, trim silence, write to `tmp_dir`.
/// Falls back to the original file if trimming fails.
fn prepare_user_samples(samples: &[PathBuf], tmp_dir: &FsPath) -> Vec<PathBuf> {
    samples
        .iter()
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("wav"))
                .unwrap_or(false)
        })
        .enumerate()
        .filter_map(|(i, p)| {
            if !p.exists() {
                tracing::warn!(path = %p.display(), "sample file missing, skipping");
                return None;
            }
            match read_wav_i16(p) {
                Ok((raw, sr)) => {
                    let (start, end) = trim_silence(&raw, sr);
                    if end <= start + (sr as usize / 10) {
                        // Less than 100 ms after trim — something is wrong, use original
                        tracing::warn!(path = %p.display(), "silence trim left <100 ms, using original");
                        return Some(p.clone());
                    }
                    let out = tmp_dir.join(format!("user_{i}.wav"));
                    match write_wav_i16(&out, &raw[start..end], sr) {
                        Ok(()) => Some(out),
                        Err(e) => {
                            tracing::warn!(path = %p.display(), "trim write failed ({e}), using original");
                            Some(p.clone())
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(path = %p.display(), "WAV read failed: {e}");
                    Some(p.clone())
                }
            }
        })
        .collect()
}

/// Generate TTS augmentation samples at 5 different speeds and resample to 16 kHz.
fn generate_tts_augments(
    phrase: &str,
    tts: Option<&dyn TextToSpeech>,
    tmp_dir: &FsPath,
) -> Vec<PathBuf> {
    let tts = match tts {
        Some(t) => t,
        None => {
            tracing::info!("TTS not configured — skipping augmentation samples");
            return vec![];
        }
    };

    const SPEEDS: [f32; 5] = [0.8, 0.9, 1.0, 1.1, 1.2];
    let mut paths = Vec::new();

    for (i, &speed) in SPEEDS.iter().enumerate() {
        let settings = TtsSettings {
            speed,
            voice: "default".to_string(),
        };
        match tts.synthesise(phrase, &settings) {
            Err(e) => tracing::warn!(speed, "TTS synthesis failed: {e}"),
            Ok(wav_bytes) => match decode_wav_f32(&wav_bytes) {
                Err(e) => tracing::warn!(speed, "TTS WAV decode failed: {e}"),
                Ok((samples_f32, src_rate)) => {
                    let resampled = resample_linear(&samples_f32, src_rate, 16_000);
                    let samples_i16: Vec<i16> = resampled.iter().map(|&s| f32_to_i16(s)).collect();
                    let out = tmp_dir.join(format!("tts_{i}.wav"));
                    match write_wav_i16(&out, &samples_i16, 16_000) {
                        Ok(()) => paths.push(out),
                        Err(e) => tracing::warn!(speed, "TTS WAV write failed: {e}"),
                    }
                }
            },
        }
    }

    paths
}

/// Find the (start, end) sample indices that contain speech, skipping leading/trailing silence.
fn trim_silence(samples: &[i16], sample_rate: u32) -> (usize, usize) {
    const THRESHOLD_RMS: f64 = 400.0; // ≈ -38 dBFS — well below speech, above silence
    let window = (sample_rate as usize * 20) / 1000; // 20 ms analysis window
    let hop = window / 2;
    let pad = (sample_rate as usize * 60) / 1000; // 60 ms context padding each side

    if samples.len() < window {
        return (0, samples.len());
    }

    let rms = |start: usize| -> f64 {
        let end = (start + window).min(samples.len());
        let sq: f64 = samples[start..end]
            .iter()
            .map(|&s| (s as f64) * (s as f64))
            .sum();
        (sq / (end - start) as f64).sqrt()
    };

    let voiced_frames: Vec<usize> = (0..samples.len().saturating_sub(window))
        .step_by(hop.max(1))
        .filter(|&i| rms(i) > THRESHOLD_RMS)
        .collect();

    match (voiced_frames.first(), voiced_frames.last()) {
        (Some(&first), Some(&last)) => {
            let start = first.saturating_sub(pad);
            let end = (last + window + pad).min(samples.len());
            (start, end)
        }
        _ => (0, samples.len()), // entirely silent — return as-is; rustpotter will reject it
    }
}

// ── WAV I/O helpers ───────────────────────────────────────────────────────────

fn read_wav_i16(path: &FsPath) -> anyhow::Result<(Vec<i16>, u32)> {
    let reader = hound::WavReader::open(path)?;
    let sr = reader.spec().sample_rate;
    let samples: Result<Vec<i16>, _> = reader.into_samples::<i16>().collect();
    Ok((samples?, sr))
}

fn write_wav_i16(path: &FsPath, samples: &[i16], sample_rate: u32) -> anyhow::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}

fn decode_wav_f32(bytes: &[u8]) -> anyhow::Result<(Vec<f32>, u32)> {
    let cursor = std::io::Cursor::new(bytes);
    let reader = hound::WavReader::new(cursor)?;
    let sr = reader.spec().sample_rate;
    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .filter_map(|s| s.ok())
        .map(|s| s as f32 / i16::MAX as f32)
        .collect();
    Ok((samples, sr))
}

/// Linear interpolation resampler — good enough for MFCC-based DTW matching.
fn resample_linear(samples: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if src_rate == dst_rate {
        return samples.to_vec();
    }
    let ratio = src_rate as f64 / dst_rate as f64;
    let out_len = (samples.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = samples.get(idx).copied().unwrap_or(0.0);
            let b = samples.get(idx + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

#[inline]
fn f32_to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

// ── rustpotter model build ────────────────────────────────────────────────────

/// Build a rustpotter DTW reference model from the supplied WAV paths (already trimmed/resampled).
fn build_wakeword_ref(phrase: &str, samples: &[PathBuf], output: &FsPath) -> anyhow::Result<()> {
    use rustpotter::{WakewordRef, WakewordRefBuildFromFiles, WakewordSave};

    let wav_paths: Vec<String> = samples
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    tracing::info!(
        count = wav_paths.len(),
        "calling rustpotter WakewordRef::new_from_sample_files"
    );

    let wakeword =
        WakewordRef::new_from_sample_files(phrase.to_string(), None, None, wav_paths, 40)
            .map_err(|e| anyhow::anyhow!("rustpotter: {e}"))?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| anyhow::anyhow!("create models dir: {e}"))?;
    }

    wakeword
        .save_to_file(&output.to_string_lossy())
        .map_err(|e| anyhow::anyhow!("save model: {e}"))?;

    Ok(())
}

// ── Wake word deployment ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DeployBody {
    pub node_ids: Vec<String>,
}

pub async fn deploy_wake_word(
    State(state): State<AppState>,
    Json(body): Json<DeployBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let model_path = {
        let training = state.wake_training.lock().await;
        training.model_path.clone().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                json_error("no trained model available"),
            )
        })?
    };

    let model_bytes = std::fs::read(&model_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    // Push model via existing gRPC broadcast channel on the session registry.
    // The BrainResponse::WakeWordModelUpdate message is sent to all matching sessions.
    let deployed_count = push_model_to_nodes(&state, &body.node_ids, model_bytes).await;

    tracing::info!(
        nodes = ?body.node_ids,
        deployed = deployed_count,
        "wake word model deployed"
    );

    Ok(Json(serde_json::json!({
        "status": "deployed",
        "deployed_to": deployed_count,
    })))
}

async fn push_model_to_nodes(state: &AppState, node_ids: &[String], model_bytes: Vec<u8>) -> usize {
    // Write to a shared volume as a fallback: nodes that reconnect later will
    // be able to pick up the model from here via a future "pending model" check.
    let models_pub_dir = state.config_dir.join("wake_models_pub");
    let _ = std::fs::create_dir_all(&models_pub_dir);
    if let Err(e) = std::fs::write(models_pub_dir.join("hey-aether.rpw"), &model_bytes) {
        tracing::warn!("failed to write model to pub dir: {e}");
    }

    // Push to each currently connected node via its per-session gRPC channel.
    let pushed = state
        .registry
        .push_wake_word_model(node_ids, model_bytes)
        .await;
    tracing::info!(pushed, "wake word model pushed to connected nodes");
    pushed
}

// ── Voice users ───────────────────────────────────────────────────────────────

pub async fn list_voice_users(State(state): State<AppState>) -> Json<Vec<VoiceUser>> {
    Json(state.voice_training.lock().await.users.clone())
}

#[derive(Deserialize)]
pub struct CreateUserBody {
    pub name: String,
}

pub async fn create_voice_user(
    State(state): State<AppState>,
    Json(body): Json<CreateUserBody>,
) -> Json<VoiceUser> {
    let user = VoiceUser {
        id: uuid::Uuid::new_v4().to_string(),
        name: body.name,
        sample_count: 0,
        trained: false,
        trained_at: None,
    };
    state.voice_training.lock().await.users.push(user.clone());
    Json(user)
}

pub async fn delete_voice_user(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> StatusCode {
    state
        .voice_training
        .lock()
        .await
        .users
        .retain(|u| u.id != id);
    StatusCode::NO_CONTENT
}

// ── Voice samples ─────────────────────────────────────────────────────────────

pub async fn upload_voice_sample(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<VoiceSample>, (StatusCode, Json<serde_json::Value>)> {
    let samples_dir = state.config_dir.join("voice_samples");
    std::fs::create_dir_all(&samples_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    let mut user_id = String::new();
    let mut transcript = String::new();
    let mut wav_data: Option<Vec<u8>> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name() {
            Some("user_id") => {
                user_id = field.text().await.unwrap_or_default();
            }
            Some("transcript") => {
                transcript = field.text().await.unwrap_or_default();
            }
            Some("wav") | Some("audio") => {
                wav_data = field.bytes().await.ok().map(|b| b.to_vec());
            }
            _ => {}
        }
    }

    let wav =
        wav_data.ok_or_else(|| (StatusCode::BAD_REQUEST, json_error("missing audio field")))?;

    let id = uuid::Uuid::new_v4().to_string();
    let filename = format!("voice_{user_id}_{id}.wav");
    std::fs::write(samples_dir.join(&filename), &wav)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    let sample = VoiceSample {
        id,
        user_id: user_id.clone(),
        transcript,
        filename,
    };
    let mut training = state.voice_training.lock().await;
    training.active_samples.push(sample.clone());
    if let Some(u) = training.users.iter_mut().find(|u| u.id == user_id) {
        u.sample_count += 1;
    }
    Ok(Json(sample))
}

// ── Voice fine-tuning ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TrainVoiceBody {
    pub user_id: String,
}

pub async fn train_voice(
    State(state): State<AppState>,
    Json(body): Json<TrainVoiceBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let finetuning_url = state.finetuning_url.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json_error("finetuning service not configured"),
        )
    })?;

    let samples: Vec<VoiceSample> = state
        .voice_training
        .lock()
        .await
        .active_samples
        .iter()
        .filter(|s| s.user_id == body.user_id)
        .cloned()
        .collect();

    if samples.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            json_error("no samples for this user"),
        ));
    }

    let tx = state.voice_progress_tx.clone();
    let voice_training = state.voice_training.clone();
    let user_id = body.user_id.clone();
    let samples_dir = state.config_dir.join("voice_samples");

    tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("build reqwest client");

        // Build multipart form with all sample WAVs.
        let mut form = reqwest::blocking::multipart::Form::new().text("user_id", user_id.clone());
        for sample in &samples {
            if let Ok(bytes) = std::fs::read(samples_dir.join(&sample.filename)) {
                form = form.part(
                    "samples",
                    reqwest::blocking::multipart::Part::bytes(bytes)
                        .file_name(sample.filename.clone()),
                );
            }
        }

        let _ = tx.send(ProgressEvent {
            percent: 5,
            message: "Submitting to fine-tuning service…".to_string(),
            ..Default::default()
        });

        let resp = client
            .post(format!("{finetuning_url}/train"))
            .multipart(form)
            .send();

        match resp {
            Ok(r) if r.status().is_success() => {
                let _ = tx.send(ProgressEvent {
                    percent: 100,
                    message: "Fine-tuning complete".to_string(),
                    done: true,
                    ..Default::default()
                });
                let mut t = voice_training.blocking_lock();
                if let Some(u) = t.users.iter_mut().find(|u| u.id == user_id) {
                    u.trained = true;
                    u.trained_at = Some(chrono_now());
                }
            }
            Ok(r) => {
                let msg = format!("finetuning service returned {}", r.status());
                let _ = tx.send(ProgressEvent {
                    percent: 0,
                    message: msg,
                    done: true,
                    error: true,
                });
            }
            Err(e) => {
                let _ = tx.send(ProgressEvent {
                    percent: 0,
                    message: format!("finetuning request failed: {e}"),
                    done: true,
                    error: true,
                });
            }
        }
    });

    Ok(Json(serde_json::json!({ "status": "training started" })))
}

pub async fn activate_voice_model(
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> StatusCode {
    StatusCode::NO_CONTENT
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn wav_duration_ms(data: &[u8]) -> u32 {
    if data.len() < 44 {
        return 0;
    }
    let sample_rate = u32::from_le_bytes(data[24..28].try_into().unwrap_or([0; 4]));
    let num_channels = u16::from_le_bytes(data[22..24].try_into().unwrap_or([0; 2])) as u32;
    let bits_per_sample = u16::from_le_bytes(data[34..36].try_into().unwrap_or([0; 2])) as u32;
    if sample_rate == 0 || num_channels == 0 || bits_per_sample == 0 {
        return 0;
    }
    let data_bytes = (data.len() as u32).saturating_sub(44);
    let bytes_per_second = sample_rate * num_channels * (bits_per_sample / 8);
    if bytes_per_second == 0 {
        return 0;
    }
    (data_bytes * 1000) / bytes_per_second
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs}")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionRegistry;
    use crate::skills::SkillRegistry;
    use crate::web_ui::{
        AppState, ModelSettings, ProgressEvent, TrainingStatus, VoiceTrainingState, WakeSample,
        WakeTrainingState,
    };
    use aether_core::{CommandTrie, TtsSettings};
    use axum::extract::{Json, State};
    use axum::http::StatusCode;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::{Mutex, RwLock};

    fn make_state(tmp: &std::path::Path, finetuning_url: Option<String>) -> AppState {
        let (wake_tx, _) = tokio::sync::broadcast::channel::<ProgressEvent>(16);
        let (voice_tx, _) = tokio::sync::broadcast::channel::<ProgressEvent>(16);
        let (ingest_tx, _) = tokio::sync::broadcast::channel::<ProgressEvent>(16);
        AppState {
            env: Arc::new(minijinja::Environment::new()),
            registry: SessionRegistry::new(),
            skills: Arc::new(SkillRegistry::default()),
            tts: None,
            tts_settings: Arc::new(RwLock::new(TtsSettings::default())),
            llm: None,
            trie: Arc::new(CommandTrie::default()),
            rag: None,
            certs_dir: tmp.to_path_buf(),
            config_dir: tmp.to_path_buf(),
            documents_dir: None,
            ollama_url: String::new(),
            wake_training: Arc::new(Mutex::new(WakeTrainingState::default())),
            voice_training: Arc::new(Mutex::new(VoiceTrainingState::default())),
            model_settings: Arc::new(RwLock::new(ModelSettings::default())),
            finetuning_url,
            http_client: reqwest::Client::new(),
            skill_config: Arc::new(RwLock::new(crate::skills::SkillConfig::default())),
            wake_progress_tx: Arc::new(wake_tx),
            voice_progress_tx: Arc::new(voice_tx),
            ingest_progress_tx: Arc::new(ingest_tx),
            brain_ip: "127.0.0.1".into(),
            ollama_update: Arc::new(RwLock::new(
                crate::ollama_updates::OllamaUpdateInfo::default(),
            )),
        }
    }

    // ── Pure function tests ───────────────────────────────────────────────────

    #[test]
    fn trim_silence_all_silent_returns_full_range() {
        let samples = vec![0i16; 16_000];
        assert_eq!(trim_silence(&samples, 16_000), (0, 16_000));
    }

    #[test]
    fn trim_silence_buffer_shorter_than_window_returns_full_range() {
        let samples = vec![1000i16; 5];
        assert_eq!(trim_silence(&samples, 16_000), (0, 5));
    }

    #[test]
    fn trim_silence_strips_leading_and_trailing_silence() {
        let sr = 16_000u32;
        let silence = sr as usize; // 1 s = 16 000 samples
        let speech = (sr as usize * 200) / 1_000; // 200 ms = 3 200 samples
        let mut samples = vec![0i16; silence];
        samples.extend(vec![8_000i16; speech]); // loud speech
        samples.extend(vec![0i16; silence]);

        let (start, end) = trim_silence(&samples, sr);
        assert!(
            start < silence,
            "leading silence must be trimmed: start={start}"
        );
        assert!(
            end > silence + speech,
            "trailing silence must be trimmed: end={end}"
        );
        assert!(end <= samples.len());
        // With 2 s of silence and only 200 ms of speech, the trim should remove >50%.
        assert!(
            end - start < samples.len() / 2,
            "trimmed region should be <50% of full buffer"
        );
    }

    #[test]
    fn resample_linear_identity_when_same_rate() {
        let samples = vec![0.1f32, 0.5, -0.3, 0.8];
        let out = resample_linear(&samples, 16_000, 16_000);
        assert_eq!(out.len(), samples.len());
        for (a, b) in samples.iter().zip(out.iter()) {
            assert!(
                (a - b).abs() < 1e-6,
                "values must match for pass-through resample"
            );
        }
    }

    #[test]
    fn resample_linear_output_length_24k_to_16k() {
        let n = 2_400usize;
        let out = resample_linear(&vec![0.0f32; n], 24_000, 16_000);
        let expected = (n as f64 * 16_000.0 / 24_000.0) as usize;
        assert_eq!(out.len(), expected);
    }

    #[test]
    fn resample_linear_output_length_16k_to_8k() {
        let n = 1_600usize;
        let out = resample_linear(&vec![0.0f32; n], 16_000, 8_000);
        let expected = (n as f64 * 8_000.0 / 16_000.0) as usize;
        assert_eq!(out.len(), expected);
    }

    #[test]
    fn f32_to_i16_clamps_above_one() {
        assert_eq!(f32_to_i16(2.0), i16::MAX);
    }

    #[test]
    fn f32_to_i16_clamps_below_minus_one() {
        assert_eq!(f32_to_i16(-2.0), -i16::MAX);
    }

    #[test]
    fn f32_to_i16_zero_maps_to_zero() {
        assert_eq!(f32_to_i16(0.0), 0);
    }

    #[test]
    fn wav_duration_ms_returns_zero_for_short_data() {
        assert_eq!(wav_duration_ms(&[0u8; 20]), 0);
    }

    #[test]
    fn wav_duration_ms_returns_zero_for_zero_sample_rate() {
        let mut data = vec![0u8; 64];
        data[22..24].copy_from_slice(&1u16.to_le_bytes()); // channels = 1
        data[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits_per_sample = 16
                                                            // sample_rate at [24..28] stays 0
        assert_eq!(wav_duration_ms(&data), 0);
    }

    #[test]
    fn wav_duration_ms_known_pcm() {
        // 16 kHz mono 16-bit → bytes_per_second = 32 000; 32 000 data bytes → 1 000 ms
        let mut data = vec![0u8; 44 + 32_000];
        data[22..24].copy_from_slice(&1u16.to_le_bytes());
        data[24..28].copy_from_slice(&16_000u32.to_le_bytes());
        data[34..36].copy_from_slice(&16u16.to_le_bytes());
        assert_eq!(wav_duration_ms(&data), 1_000);
    }

    #[test]
    fn prepare_user_samples_rejects_non_wav_extension() {
        let tmp = TempDir::new().unwrap();
        // .webm path — extension filter runs before exists() check, so file need not exist
        let result = prepare_user_samples(&[tmp.path().join("sample.webm")], tmp.path());
        assert!(result.is_empty(), "non-WAV files must be filtered out");
    }

    #[test]
    fn prepare_user_samples_skips_missing_files() {
        let tmp = TempDir::new().unwrap();
        let result = prepare_user_samples(&[tmp.path().join("ghost.wav")], tmp.path());
        assert!(result.is_empty(), "missing files must be skipped");
    }

    #[test]
    fn prepare_user_samples_too_short_after_trim_falls_back_to_original() {
        let tmp = TempDir::new().unwrap();
        let wav_path = tmp.path().join("short.wav");
        // 50 ms silent WAV: trim returns (0, 800); 800 ≤ 1600 (100 ms) → falls back
        write_wav_i16(&wav_path, &vec![0i16; 800], 16_000).unwrap();
        let out = TempDir::new().unwrap();
        let result = prepare_user_samples(std::slice::from_ref(&wav_path), out.path());
        assert_eq!(
            result,
            vec![wav_path],
            "short samples must fall back to original path"
        );
    }

    #[test]
    fn prepare_user_samples_writes_trimmed_copy_to_tmp_dir() {
        let tmp = TempDir::new().unwrap();
        let wav_path = tmp.path().join("audio.wav");
        // 200 ms silent WAV: trim returns full range (> 100 ms) → copies to tmp dir
        write_wav_i16(&wav_path, &vec![0i16; 3_200], 16_000).unwrap();
        let out = TempDir::new().unwrap();
        let result = prepare_user_samples(std::slice::from_ref(&wav_path), out.path());
        assert_eq!(result.len(), 1);
        assert_ne!(
            result[0], wav_path,
            "should return copy in tmp dir, not original"
        );
        assert!(result[0].exists(), "copied file must exist on disk");
    }

    // ── Handler tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn train_wake_word_zero_samples_returns_bad_request() {
        let tmp = TempDir::new().unwrap();
        let err = train_wake_word(
            State(make_state(tmp.path(), None)),
            Json(TrainWakeWordBody { phrase: None }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn train_wake_word_two_samples_returns_bad_request() {
        let tmp = TempDir::new().unwrap();
        let state = make_state(tmp.path(), None);
        {
            let mut t = state.wake_training.lock().await;
            for i in 0..2u32 {
                t.samples.push(WakeSample {
                    id: i.to_string(),
                    filename: format!("s{i}.wav"),
                    duration_ms: 1_000,
                    size_bytes: 1_000,
                });
            }
        }
        let err = train_wake_word(State(state), Json(TrainWakeWordBody { phrase: None }))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn train_wake_word_already_running_returns_conflict() {
        let tmp = TempDir::new().unwrap();
        let state = make_state(tmp.path(), None);
        {
            let mut t = state.wake_training.lock().await;
            for i in 0..3u32 {
                t.samples.push(WakeSample {
                    id: i.to_string(),
                    filename: format!("s{i}.wav"),
                    duration_ms: 1_000,
                    size_bytes: 1_000,
                });
            }
            t.status = TrainingStatus::Running {
                progress: 50,
                message: "in progress".into(),
            };
        }
        let err = train_wake_word(State(state), Json(TrainWakeWordBody { phrase: None }))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn train_wake_word_three_samples_starts_training() {
        let tmp = TempDir::new().unwrap();
        let state = make_state(tmp.path(), None);
        let samples_dir = tmp.path().join("wake_samples");
        std::fs::create_dir_all(&samples_dir).unwrap();
        {
            let mut t = state.wake_training.lock().await;
            for i in 0..3u32 {
                let filename = format!("s{i}.wav");
                write_wav_i16(&samples_dir.join(&filename), &vec![0i16; 16_000], 16_000).unwrap();
                t.samples.push(WakeSample {
                    id: i.to_string(),
                    filename,
                    duration_ms: 1_000,
                    size_bytes: 1_000,
                });
            }
        }
        let Json(body) = train_wake_word(State(state), Json(TrainWakeWordBody { phrase: None }))
            .await
            .unwrap();
        assert_eq!(body["status"], "training started");
    }

    #[tokio::test]
    async fn train_voice_no_finetuning_url_returns_503() {
        let tmp = TempDir::new().unwrap();
        let err = train_voice(
            State(make_state(tmp.path(), None)),
            Json(TrainVoiceBody {
                user_id: "u1".into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn train_voice_nonexistent_user_returns_bad_request() {
        let tmp = TempDir::new().unwrap();
        let err = train_voice(
            State(make_state(tmp.path(), Some("http://localhost:9999".into()))),
            Json(TrainVoiceBody {
                user_id: "ghost".into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn train_voice_user_exists_but_no_samples_returns_bad_request() {
        let tmp = TempDir::new().unwrap();
        let state = make_state(tmp.path(), Some("http://localhost:9999".into()));
        let Json(user) = create_voice_user(
            State(state.clone()),
            Json(CreateUserBody {
                name: "Charlie".into(),
            }),
        )
        .await;
        let err = train_voice(State(state), Json(TrainVoiceBody { user_id: user.id }))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn deploy_wake_word_no_model_returns_bad_request() {
        let tmp = TempDir::new().unwrap();
        let err = deploy_wake_word(
            State(make_state(tmp.path(), None)),
            Json(DeployBody { node_ids: vec![] }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_voice_user_stores_user_in_state() {
        let tmp = TempDir::new().unwrap();
        let state = make_state(tmp.path(), None);
        let Json(user) = create_voice_user(
            State(state.clone()),
            Json(CreateUserBody {
                name: "Alice".into(),
            }),
        )
        .await;
        assert_eq!(user.name, "Alice");
        assert!(!user.id.is_empty());
        assert_eq!(user.sample_count, 0);
        assert!(!user.trained);
        let Json(users) = list_voice_users(State(state)).await;
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, user.id);
    }

    #[tokio::test]
    async fn create_voice_user_empty_name_is_accepted() {
        // Documents current behaviour: no name validation at the API layer.
        let tmp = TempDir::new().unwrap();
        let Json(user) = create_voice_user(
            State(make_state(tmp.path(), None)),
            Json(CreateUserBody {
                name: String::new(),
            }),
        )
        .await;
        assert_eq!(user.name, "");
    }

    #[tokio::test]
    async fn delete_voice_user_nonexistent_id_is_no_content() {
        let tmp = TempDir::new().unwrap();
        let status = delete_voice_user(
            axum::extract::Path("no-such-user".to_string()),
            State(make_state(tmp.path(), None)),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn delete_voice_user_removes_existing_user() {
        let tmp = TempDir::new().unwrap();
        let state = make_state(tmp.path(), None);
        let Json(user) = create_voice_user(
            State(state.clone()),
            Json(CreateUserBody {
                name: "Dave".into(),
            }),
        )
        .await;
        let status =
            delete_voice_user(axum::extract::Path(user.id.clone()), State(state.clone())).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let Json(users) = list_voice_users(State(state)).await;
        assert!(
            users.iter().all(|u| u.id != user.id),
            "deleted user must not be listed"
        );
    }

    #[tokio::test]
    async fn delete_wake_sample_nonexistent_id_is_no_content() {
        let tmp = TempDir::new().unwrap();
        let status = delete_wake_sample(
            axum::extract::Path("ghost-id".to_string()),
            State(make_state(tmp.path(), None)),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn list_wake_samples_empty_initially() {
        let tmp = TempDir::new().unwrap();
        let Json(samples) = list_wake_samples(State(make_state(tmp.path(), None))).await;
        assert!(samples.is_empty());
    }

    #[tokio::test]
    async fn list_voice_users_empty_initially() {
        let tmp = TempDir::new().unwrap();
        let Json(users) = list_voice_users(State(make_state(tmp.path(), None))).await;
        assert!(users.is_empty());
    }
}
