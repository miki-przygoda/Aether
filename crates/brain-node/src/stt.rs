use anyhow::{Context, Result};
use std::sync::Arc;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct TranscriptResult {
    pub text: String,
    pub confidence: f32,
}

/// Abstraction over STT backends — lets tests inject a mock without a real model.
pub trait SpeechToText: Send + Sync {
    fn transcribe(&self, pcm: &[f32]) -> Result<TranscriptResult>;
}

pub struct WhisperStt {
    primary: Arc<WhisperContext>,
    fallback: Option<Arc<WhisperContext>>,
    pub confidence_threshold: f32,
}

impl WhisperStt {
    pub fn new(
        model_path: &str,
        fallback_path: Option<&str>,
        confidence_threshold: f32,
    ) -> Result<Self> {
        let primary = Arc::new(
            WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
                .with_context(|| format!("loading Whisper model from {model_path}"))?,
        );
        let fallback = fallback_path
            .map(|p| {
                WhisperContext::new_with_params(p, WhisperContextParameters::default())
                    .map(Arc::new)
                    .with_context(|| format!("loading fallback Whisper model from {p}"))
            })
            .transpose()?;
        Ok(Self {
            primary,
            fallback,
            confidence_threshold,
        })
    }
}

impl SpeechToText for WhisperStt {
    fn transcribe(&self, pcm: &[f32]) -> Result<TranscriptResult> {
        let result = run_inference(&self.primary, pcm)?;
        if result.confidence < self.confidence_threshold {
            if let Some(fb) = &self.fallback {
                tracing::debug!(
                    confidence = result.confidence,
                    threshold = self.confidence_threshold,
                    "low confidence — re-running with fallback model"
                );
                return run_inference(fb, pcm);
            }
        }
        Ok(result)
    }
}

fn run_inference(ctx: &WhisperContext, pcm: &[f32]) -> Result<TranscriptResult> {
    let mut state = ctx.create_state().context("creating Whisper state")?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_print_realtime(false);
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);

    state.full(params, pcm).context("Whisper inference")?;

    let n_segments = state.full_n_segments().context("getting segment count")?;
    let mut text = String::new();
    let mut total_prob = 0.0f32;
    let mut token_count = 0i32;

    for seg in 0..n_segments {
        text.push_str(
            &state
                .full_get_segment_text(seg)
                .context("getting segment text")?,
        );
        let n_tokens = state.full_n_tokens(seg).context("getting token count")?;
        for tok in 0..n_tokens {
            if let Ok(data) = state.full_get_token_data(seg, tok) {
                total_prob += data.p;
                token_count += 1;
            }
        }
    }

    let confidence = if token_count > 0 {
        total_prob / token_count as f32
    } else {
        0.0
    };

    Ok(TranscriptResult {
        text: text.trim().to_string(),
        confidence,
    })
}

/// Decode raw f32le bytes (as stored in `AudioChunk.pcm`) into f32 samples.
pub fn bytes_to_f32le(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().expect("chunks_exact guarantees 4 bytes")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_f32le_roundtrips() {
        let samples = [0.0f32, 0.5, -0.5, 1.0];
        let bytes: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();
        let decoded = bytes_to_f32le(&bytes);
        for (a, b) in samples.iter().zip(decoded.iter()) {
            assert!((a - b).abs() < 1e-6, "{a} != {b}");
        }
    }

    #[test]
    fn bytes_to_f32le_ignores_trailing_byte() {
        let bytes = vec![0u8; 9]; // 2 complete f32s + 1 leftover byte
        assert_eq!(bytes_to_f32le(&bytes).len(), 2);
    }

    #[test]
    #[ignore = "requires Whisper model — set WHISPER_MODEL_PATH to run"]
    fn real_model_loads_and_transcribes_silence() {
        let path = std::env::var("WHISPER_MODEL_PATH").expect("WHISPER_MODEL_PATH not set");
        let stt = WhisperStt::new(&path, None, 0.75).expect("model load failed");
        let pcm = vec![0.0f32; 16_000]; // 1 s of silence @ 16 kHz
        let result = stt.transcribe(&pcm).expect("transcribe failed");
        assert!(
            result.text.len() < 100,
            "silence should not produce a long transcript, got: {:?}",
            result.text
        );
    }
}
