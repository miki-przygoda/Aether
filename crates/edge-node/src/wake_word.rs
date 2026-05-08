use aether_core::wake_word::WakeWordDetector;
use anyhow::Result;
use std::path::Path;

/// Build the active wake word detector.
///
/// With the `wake-word` feature (Pi production builds):
///   Uses rustpotter with a trained .rpw model. Fails fast if the model is missing.
///
/// Without the feature (CI / dev builds):
///   Uses DevDetector — always-silent, never triggers from audio.
///   The audio pipeline still runs so you can verify the gRPC stream end-to-end.
pub fn build(model_path: &Path) -> Result<Box<dyn WakeWordDetector>> {
    #[cfg(feature = "wake-word")]
    {
        let det = RustpotterDetector::from_model(model_path)?;
        Ok(Box::new(det))
    }
    #[cfg(not(feature = "wake-word"))]
    {
        let _ = model_path;
        tracing::warn!(
            "compiled without `wake-word` feature — using DevDetector (never triggers). \
             Rebuild with `--features wake-word` for a production Pi build."
        );
        Ok(Box::new(DevDetector))
    }
}

// ─── real detector (rustpotter) ─────────────────────────────────────────────

#[cfg(feature = "wake-word")]
mod rustpotter_impl {
    use super::*;
    use anyhow::Context;
    use rustpotter::{Rustpotter, RustpotterConfig};

    pub struct RustpotterDetector {
        inner: Rustpotter,
        frame_size: usize,
        buffer: Vec<f32>,
    }

    impl RustpotterDetector {
        pub fn from_model(model_path: &Path) -> Result<Self> {
            if !model_path.exists() {
                anyhow::bail!(
                    "wake word model not found at '{}'\n\
                     Run the training wizard (Phase 5 web UI) or place a pre-trained \
                     .rpw file at that path before starting.",
                    model_path.display()
                );
            }
            let config = RustpotterConfig::default();
            let mut inner = Rustpotter::new(&config).context("failed to initialise rustpotter")?;
            inner
                .add_wakeword_from_file("hey-aether", model_path)
                .context("failed to load wake word model")?;
            let frame_size = inner.get_samples_per_frame();
            tracing::info!(model = ?model_path, frame_size, "wake word model loaded");
            Ok(Self {
                inner,
                frame_size,
                buffer: Vec::with_capacity(frame_size * 2),
            })
        }
    }

    impl WakeWordDetector for RustpotterDetector {
        fn process_samples(&mut self, samples: &[f32]) -> bool {
            self.buffer.extend_from_slice(samples);
            let mut detected = false;
            while self.buffer.len() >= self.frame_size {
                let frame: Vec<f32> = self.buffer.drain(..self.frame_size).collect();
                if self.inner.process_samples(frame).is_some() {
                    detected = true;
                }
            }
            detected
        }
    }
}

#[cfg(feature = "wake-word")]
pub use rustpotter_impl::RustpotterDetector;

// ─── development stub ────────────────────────────────────────────────────────

/// Never triggers from audio samples. Lets the pipeline compile and run in CI
/// without a trained wake word model or the rustpotter feature enabled.
pub struct DevDetector;

impl WakeWordDetector for DevDetector {
    fn process_samples(&mut self, _samples: &[f32]) -> bool {
        false
    }
}
