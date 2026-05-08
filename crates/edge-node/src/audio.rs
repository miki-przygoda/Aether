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
