/// Abstraction over wake word backends.  Swap rustpotter for anything else
/// without touching the audio pipeline.
pub trait WakeWordDetector: Send {
    /// Feed a chunk of f32 PCM samples (16 kHz, mono).
    /// Returns `true` the moment the wake phrase is detected.
    fn process_samples(&mut self, samples: &[f32]) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysTrigger;
    impl WakeWordDetector for AlwaysTrigger {
        fn process_samples(&mut self, _: &[f32]) -> bool {
            true
        }
    }

    struct NeverTrigger;
    impl WakeWordDetector for NeverTrigger {
        fn process_samples(&mut self, _: &[f32]) -> bool {
            false
        }
    }

    #[test]
    fn trait_mock_trigger() {
        let mut det: Box<dyn WakeWordDetector> = Box::new(AlwaysTrigger);
        assert!(det.process_samples(&[0.0f32; 480]));
    }

    #[test]
    fn trait_mock_no_trigger() {
        let mut det: Box<dyn WakeWordDetector> = Box::new(NeverTrigger);
        assert!(!det.process_samples(&[0.0f32; 480]));
    }
}
