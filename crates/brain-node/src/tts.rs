use aether_core::TtsSettings;
use anyhow::{Context, Result};
use ort::{session::builder::GraphOptimizationLevel, session::Session, value::Tensor};
use std::collections::HashMap;
use std::io::Cursor;

/// Abstraction over TTS backends — allows mock injection in tests.
pub trait TextToSpeech: Send + Sync {
    /// Synthesise `text` with the given settings and return WAV bytes (24 kHz, mono, 16-bit PCM).
    fn synthesise(&self, text: &str, settings: &TtsSettings) -> Result<Vec<u8>>;
}

// ─── Kokoro-82M ONNX ─────────────────────────────────────────────────────────

const SAMPLE_RATE: u32 = 24_000;

pub struct KokoroTts {
    session: std::sync::Mutex<Session>,
    /// Phoneme character → Kokoro token ID (loaded from vocab.json).
    vocab: HashMap<char, i64>,
    /// 256 float32 values for the default voice style.
    style: Vec<f32>,
}

impl KokoroTts {
    /// Load a Kokoro-82M ONNX model.
    ///
    /// Expects two companion files in the same directory as `model_path`:
    ///   - `vocab.json`       — JSON object mapping phoneme chars to i64 token IDs
    ///   - `voice_style.bin`  — 1024 raw bytes: 256 × f32le default voice embedding
    pub fn new(model_path: &str) -> Result<Self> {
        let model_dir = std::path::Path::new(model_path)
            .parent()
            .context("model path has no parent directory")?;

        let session = Session::builder()
            .context("creating ORT session builder")?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow::anyhow!("setting optimization level: {e}"))?
            .commit_from_file(model_path)
            .context("loading Kokoro ONNX model")?;

        let vocab: HashMap<char, i64> = serde_json::from_str(
            &std::fs::read_to_string(model_dir.join("vocab.json"))
                .context("reading vocab.json alongside model")?,
        )
        .context("parsing vocab.json")?;

        let style_bytes = std::fs::read(model_dir.join("voice_style.bin"))
            .context("reading voice_style.bin (256 × f32le)")?;
        anyhow::ensure!(
            style_bytes.len() == 256 * 4,
            "voice_style.bin must be exactly {} bytes (256 × f32le), got {}",
            256 * 4,
            style_bytes.len()
        );
        let style = style_bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().expect("chunk is 4 bytes")))
            .collect();

        Ok(Self {
            session: std::sync::Mutex::new(session),
            vocab,
            style,
        })
    }
}

impl KokoroTts {
    /// Synthesise `text` at the given playback `speed` (1.0 = normal, 0.8 = slower, 1.2 = faster).
    /// Useful for generating training samples with varied pace.
    pub fn synthesise_at_speed(&self, text: &str, speed: f32) -> Result<Vec<u8>> {
        let phonemes = phonemize(text).context("phonemizing text via espeak-ng")?;
        let token_ids = phonemes_to_ids(&phonemes, &self.vocab);

        let tokens: Vec<i64> = std::iter::once(0)
            .chain(token_ids)
            .chain(std::iter::once(0))
            .collect();
        let seq_len = tokens.len();

        let style_data: Vec<f32> = self
            .style
            .iter()
            .copied()
            .cycle()
            .take(seq_len * 256)
            .collect();

        let tokens_tensor =
            Tensor::<i64>::from_array(([1, seq_len], tokens)).context("building tokens tensor")?;
        let style_tensor = Tensor::<f32>::from_array(([1, seq_len, 256], style_data))
            .context("building style tensor")?;
        let speed_tensor = Tensor::<f32>::from_array(([] as [usize; 0], vec![speed]))
            .context("building speed tensor")?;

        let mut sess = self.session.lock().expect("session mutex poisoned");
        let outputs = sess
            .run(ort::inputs![
                "tokens" => tokens_tensor,
                "style"  => style_tensor,
                "speed"  => speed_tensor,
            ])
            .context("running Kokoro ONNX inference")?;

        let (_shape, samples) = outputs[0]
            .try_extract_tensor::<f32>()
            .context("extracting audio tensor from Kokoro output")?;

        encode_wav(samples, SAMPLE_RATE).context("encoding WAV")
    }
}

impl TextToSpeech for KokoroTts {
    fn synthesise(&self, text: &str, settings: &TtsSettings) -> Result<Vec<u8>> {
        self.synthesise_at_speed(text, settings.speed)
    }
}

// ─── Phonemizer ───────────────────────────────────────────────────────────────

/// Convert text to IPA phonemes using espeak-ng (must be installed in the environment).
fn phonemize(text: &str) -> Result<String> {
    let out = std::process::Command::new("espeak-ng")
        .args(["-v", "en-us", "-q", "--ipa"])
        .arg(text)
        .output()
        .context("spawning espeak-ng — is espeak-ng installed?")?;

    anyhow::ensure!(
        out.status.success(),
        "espeak-ng exited with {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    Ok(String::from_utf8(out.stdout)
        .context("espeak-ng output is not valid UTF-8")?
        .trim()
        .to_string())
}

/// Map IPA phoneme string to Kokoro token IDs, skipping unknown characters.
pub fn phonemes_to_ids(phonemes: &str, vocab: &HashMap<char, i64>) -> Vec<i64> {
    phonemes
        .chars()
        .filter_map(|c| vocab.get(&c).copied())
        .collect()
}

// ─── WAV encoding ─────────────────────────────────────────────────────────────

/// Encode f32 PCM samples (clamped to [-1, 1]) as 16-bit PCM WAV in memory.
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::with_capacity(samples.len() * 2 + 44));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(&mut buf, spec).context("creating WAV writer")?;
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(v).context("writing WAV sample")?;
    }
    writer.finalize().context("finalizing WAV")?;
    Ok(buf.into_inner())
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_wav_produces_valid_header() {
        let samples = vec![0.0f32; 100];
        let wav = encode_wav(&samples, 24_000).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn encode_wav_clamps_samples() {
        // Oversaturated samples should not panic.
        let wav = encode_wav(&[2.0, -2.0], 24_000).unwrap();
        assert!(!wav.is_empty());
    }

    #[test]
    fn phonemes_to_ids_maps_known_chars() {
        let vocab: HashMap<char, i64> = [('æ', 38), ('ɪ', 44), (' ', 4)].into_iter().collect();
        let ids = phonemes_to_ids("æ ɪ", &vocab);
        assert_eq!(ids, vec![38, 4, 44]);
    }

    #[test]
    fn phonemes_to_ids_skips_unknown_chars() {
        let vocab: HashMap<char, i64> = [('a', 1)].into_iter().collect();
        let ids = phonemes_to_ids("aXa", &vocab);
        assert_eq!(ids, vec![1, 1]);
    }

    #[test]
    #[ignore = "requires Kokoro ONNX model — set KOKORO_MODEL_PATH to run"]
    fn live_kokoro_synthesises_non_empty_wav() {
        let path = std::env::var("KOKORO_MODEL_PATH").expect("KOKORO_MODEL_PATH must be set");
        let tts = KokoroTts::new(&path).unwrap();
        let wav = tts
            .synthesise("hello world", &TtsSettings::default())
            .unwrap();
        assert!(wav.len() > 44, "WAV should have more than just a header");
        assert_eq!(&wav[0..4], b"RIFF");
    }
}
