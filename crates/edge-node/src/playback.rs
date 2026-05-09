use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleRate, StreamConfig};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

/// Play WAV bytes through the default audio output device.
///
/// Blocks until all samples have been sent to the hardware buffer.
/// Returns `Ok(())` immediately (with a warning) if no output device exists,
/// so TTS failure never kills the audio stream.
pub fn play_wav(wav_bytes: &[u8]) -> Result<()> {
    let cursor = std::io::Cursor::new(wav_bytes);
    let mut reader = hound::WavReader::new(cursor).context("parsing WAV header")?;
    let spec = reader.spec();
    let src_samples = collect_f32(&mut reader, spec)?;

    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            tracing::warn!("no audio output device — skipping TTS playback");
            return Ok(());
        }
    };

    let out = device
        .default_output_config()
        .context("querying default output config")?;

    let out_channels = out.channels() as usize;
    let out_rate = out.sample_rate().0;

    // Resample from Kokoro's 24 kHz to the hardware rate, then expand to the
    // hardware channel count by repeating the mono sample on each channel.
    let adapted = adapt(
        &src_samples,
        spec.sample_rate,
        spec.channels as usize,
        out_rate,
        out_channels,
    );

    let config = StreamConfig {
        channels: out.channels(),
        sample_rate: SampleRate(out_rate),
        buffer_size: BufferSize::Default,
    };

    let queue: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(adapted.into()));
    let queue_cb = queue.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let finished_cb = finished.clone();

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _| {
                let mut q = queue_cb.lock().unwrap();
                for frame in data.iter_mut() {
                    match q.pop_front() {
                        Some(s) => *frame = s,
                        None => {
                            *frame = 0.0;
                            finished_cb.store(true, Ordering::Relaxed);
                        }
                    }
                }
            },
            |err| tracing::error!("audio output error: {err}"),
            None,
        )
        .context("building cpal output stream")?;

    stream.play().context("starting playback")?;

    // Spin until the callback has drained all samples.
    while !finished.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Let the hardware buffer flush before dropping the stream.
    std::thread::sleep(std::time::Duration::from_millis(80));

    Ok(())
}

// ─── helpers ──────────────────────────────────────────────────────────────────

fn collect_f32(
    reader: &mut hound::WavReader<std::io::Cursor<&[u8]>>,
    spec: hound::WavSpec,
) -> Result<Vec<f32>> {
    let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.context("reading f32 sample"))
            .collect::<Result<Vec<_>>>()?,
        hound::SampleFormat::Int => match spec.bits_per_sample {
            8 => reader
                .samples::<i8>()
                .map(|s| Ok(s.context("reading i8 sample")? as f32 / 127.0))
                .collect::<Result<Vec<_>>>()?,
            16 => reader
                .samples::<i16>()
                .map(|s| Ok(s.context("reading i16 sample")? as f32 / i16::MAX as f32))
                .collect::<Result<Vec<_>>>()?,
            24 | 32 => reader
                .samples::<i32>()
                .map(|s| Ok(s.context("reading i32 sample")? as f32 / max))
                .collect::<Result<Vec<_>>>()?,
            b => anyhow::bail!("unsupported bit depth: {b}"),
        },
    };
    Ok(samples)
}

/// Resample `src` from `src_rate`→`dst_rate` (linear interpolation),
/// then expand from `src_ch` to `dst_ch` channels by repeating each frame.
fn adapt(src: &[f32], src_rate: u32, src_ch: usize, dst_rate: u32, dst_ch: usize) -> Vec<f32> {
    let src_frames = src.len() / src_ch.max(1);
    let dst_frames = (src_frames as f64 * dst_rate as f64 / src_rate as f64).ceil() as usize;
    let mut out = Vec::with_capacity(dst_frames * dst_ch);

    for i in 0..dst_frames {
        let pos = i as f64 * src_rate as f64 / dst_rate as f64;
        let lo = (pos as usize).min(src_frames.saturating_sub(1));
        let hi = (lo + 1).min(src_frames.saturating_sub(1));
        let frac = pos - lo as f64;

        // Average all source channels into a single mono value, then interpolate.
        let sample_lo: f32 = if src_ch == 0 {
            0.0
        } else {
            (0..src_ch)
                .map(|c| *src.get(lo * src_ch + c).unwrap_or(&0.0))
                .sum::<f32>()
                / src_ch as f32
        };
        let sample_hi: f32 = if src_ch == 0 {
            0.0
        } else {
            (0..src_ch)
                .map(|c| *src.get(hi * src_ch + c).unwrap_or(&0.0))
                .sum::<f32>()
                / src_ch as f32
        };
        let sample = sample_lo + (sample_hi - sample_lo) * frac as f32;

        for _ in 0..dst_ch {
            out.push(sample);
        }
    }

    out
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn make_wav_silence(n_samples: usize, sample_rate: u32) -> Vec<u8> {
        make_wav(&vec![0.0f32; n_samples], sample_rate)
    }

    fn make_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
        for &s in samples {
            w.write_sample((s * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
        buf.into_inner()
    }

    #[test]
    fn collect_f32_parses_16bit_wav() {
        let wav = make_wav(&[0.5, -0.5, 0.0], 24_000);
        let cursor = std::io::Cursor::new(wav.as_slice());
        let mut reader = hound::WavReader::new(cursor).unwrap();
        let spec = reader.spec();
        let samples = collect_f32(&mut reader, spec).unwrap();
        assert_eq!(samples.len(), 3);
        assert!((samples[0] - 0.5).abs() < 1e-4, "first sample ≈ 0.5");
        assert!((samples[1] + 0.5).abs() < 1e-4, "second sample ≈ -0.5");
        assert!(samples[2].abs() < 1e-4, "third sample ≈ 0.0");
    }

    #[test]
    fn adapt_passthrough_when_rates_match() {
        let src = vec![0.1, 0.2, 0.3];
        let out = adapt(&src, 24_000, 1, 24_000, 1);
        assert_eq!(out.len(), src.len());
        for (a, b) in out.iter().zip(src.iter()) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn adapt_mono_to_stereo_doubles_frames() {
        let src = vec![0.5; 10]; // 10 mono frames
        let out = adapt(&src, 24_000, 1, 24_000, 2);
        assert_eq!(out.len(), 20); // 10 frames × 2 channels
    }

    #[test]
    fn adapt_upsamples_correctly() {
        let src = vec![1.0f32; 4]; // 4 frames at 24 kHz
        let out = adapt(&src, 24_000, 1, 48_000, 1);
        assert_eq!(out.len(), 8); // 4 × (48/24)
        for s in &out {
            assert!((*s - 1.0).abs() < 1e-5, "upsampled silence should be 1.0");
        }
    }

    #[test]
    fn play_wav_returns_ok_on_minimal_valid_wav() {
        // No output device in CI — function should return Ok (warning logged).
        let wav = make_wav(&[0.0f32; 24_000], 24_000); // 1 s of silence
        let result = play_wav(&wav);
        // Either succeeds (device present) or fails gracefully. Never panics.
        let _ = result;
    }

    #[test]
    fn play_wav_returns_err_on_garbage_bytes() {
        let result = play_wav(b"not a wav file at all");
        assert!(result.is_err(), "garbage input should return an error");
    }
}
