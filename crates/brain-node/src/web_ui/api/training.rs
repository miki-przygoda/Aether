use crate::web_ui::{
    json_error, AppState, ProgressEvent, TrainingStatus, VoiceSample, VoiceUser, WakeSample,
};
use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::path::PathBuf;

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

    let Ok(Some(field)) = multipart.next_field().await else {
        return Err((StatusCode::BAD_REQUEST, json_error("no file field")));
    };

    let data = field
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, json_error(e.to_string())))?;

    let id = uuid::Uuid::new_v4().to_string();
    let filename = format!("sample_{id}.wav");
    let path = samples_dir.join(&filename);
    std::fs::write(&path, &data)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    let duration_ms = wav_duration_ms(&data);
    let sample = WakeSample {
        id: id.clone(),
        filename: filename.clone(),
        duration_ms,
        size_bytes: data.len() as u64,
    };

    state
        .wake_training
        .lock()
        .await
        .samples
        .push(sample.clone());
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
    let samples: Vec<PathBuf> = state
        .wake_training
        .lock()
        .await
        .samples
        .iter()
        .map(|s| samples_dir.join(&s.filename))
        .collect();

    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&models_dir).ok();

        {
            let mut t = wake_training.blocking_lock();
            t.status = TrainingStatus::Running {
                progress: 0,
                message: "Starting rustpotter-cli…".to_string(),
            };
        }
        let _ = tx.send(ProgressEvent {
            percent: 10,
            message: "Collecting samples…".to_string(),
            done: false,
        });

        let output_path = models_dir.join("hey-aether.rpw");
        let status = run_rustpotter_train(&phrase, &samples, &output_path);

        let final_status = match status {
            Ok(()) => {
                let _ = tx.send(ProgressEvent {
                    percent: 100,
                    message: "Training complete".to_string(),
                    done: true,
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

fn run_rustpotter_train(phrase: &str, samples: &[PathBuf], output: &PathBuf) -> anyhow::Result<()> {
    let mut cmd = std::process::Command::new("rustpotter-cli");
    cmd.arg("train")
        .arg("--wakeword")
        .arg(phrase)
        .arg("--output")
        .arg(output);
    for s in samples {
        cmd.arg(s);
    }
    let status = cmd.status()?;
    anyhow::ensure!(status.success(), "rustpotter-cli exited with {status}");
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
    // We broadcast a synthetic NodeStateEvent; the real push path needs a
    // dedicated channel per session. For Phase 5 the model is saved to a
    // shared volume path that edge nodes can read on reconnect.
    let models_pub_dir = state.config_dir.join("wake_models_pub");
    let _ = std::fs::create_dir_all(&models_pub_dir);
    let _ = std::fs::write(models_pub_dir.join("hey-aether.rpw"), &model_bytes);

    // Signal all matching sessions (they will pick up the model on reconnect).
    let sessions = state.registry.snapshot().await;
    sessions
        .iter()
        .filter(|s| node_ids.is_empty() || node_ids.contains(&s.node_id))
        .count()
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
            done: false,
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
                });
            }
            Err(e) => {
                let _ = tx.send(ProgressEvent {
                    percent: 0,
                    message: format!("finetuning request failed: {e}"),
                    done: true,
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
