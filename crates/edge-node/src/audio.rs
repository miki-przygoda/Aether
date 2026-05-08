use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig};
use tokio::sync::mpsc;

const SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u16 = 1;

/// Spawn a cpal audio capture stream and return a channel of f32 sample chunks.
/// Each chunk is 512 samples (32 ms at 16 kHz).
pub fn start_capture(tx: mpsc::Sender<Vec<f32>>) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .context("no audio input device found")?;

    tracing::info!(device = ?device.name(), "audio capture device");

    let supported = device
        .default_input_config()
        .context("no supported input config")?;

    let config = StreamConfig {
        channels: CHANNELS,
        sample_rate: SampleRate(SAMPLE_RATE),
        buffer_size: cpal::BufferSize::Default,
    };

    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, tx, |s| s)?,
        SampleFormat::I16 => {
            build_stream::<i16>(&device, &config, tx, |s| s as f32 / i16::MAX as f32)?
        }
        SampleFormat::U16 => build_stream::<u16>(&device, &config, tx, |s| {
            (s as f32 / u16::MAX as f32) * 2.0 - 1.0
        })?,
        fmt => anyhow::bail!("unsupported sample format: {fmt:?}"),
    };

    stream.play().context("starting audio stream")?;
    Ok(stream)
}

// Device-specific validation (mic present, correct ALSA device, acceptable noise
// floor) cannot be unit-tested meaningfully — cpal::default_input_device() returns
// whatever the OS considers default, which differs per machine.  The right place
// for this is a debug/diagnostic TUI added in a later phase once the full stack
// is running end-to-end.  At that point a live VU meter and loopback test will
// catch hardware issues far more reliably than any automated test can.

#[cfg(test)]
mod tests {
    /// I16 → f32: 0 → 0.0, MAX → ≈ 1.0, MIN → just below -1.0
    #[test]
    fn i16_to_f32_boundaries() {
        let cvt = |s: i16| s as f32 / i16::MAX as f32;
        assert_eq!(cvt(0), 0.0);
        assert!(
            (cvt(i16::MAX) - 1.0).abs() < 1e-5,
            "i16::MAX should map to ≈1.0"
        );
        // i16::MIN is -32768, MAX is 32767, so ratio is slightly beyond -1.0.
        assert!(cvt(i16::MIN) < -1.0 + 1e-4);
    }

    /// U16 → f32: 0 → -1.0, MAX → 1.0, midpoint → near 0.0
    #[test]
    fn u16_to_f32_boundaries() {
        let cvt = |s: u16| (s as f32 / u16::MAX as f32) * 2.0 - 1.0;
        assert!(
            (cvt(0) - (-1.0)).abs() < 1e-5,
            "u16 zero should map to -1.0"
        );
        assert!(
            (cvt(u16::MAX) - 1.0).abs() < 1e-5,
            "u16 MAX should map to 1.0"
        );
        let mid = cvt(u16::MAX / 2);
        assert!(
            mid.abs() < 2e-4,
            "u16 midpoint should be near 0.0, got {mid}"
        );
    }

    /// F32 samples pass through unchanged (identity convert).
    #[test]
    fn f32_passthrough_is_identity() {
        let cvt = |s: f32| s;
        assert_eq!(cvt(0.5_f32), 0.5);
        assert_eq!(cvt(-1.0_f32), -1.0);
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    tx: mpsc::Sender<Vec<f32>>,
    convert: fn(T) -> f32,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + Send + 'static,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let samples: Vec<f32> = data.iter().copied().map(convert).collect();
            // Non-blocking send; drop chunks if the consumer falls behind.
            let _ = tx.try_send(samples);
        },
        |err| tracing::error!("audio stream error: {err}"),
        None,
    )?;
    Ok(stream)
}
