use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::sync::mpsc;

use strim_shared::{AudioConfig, AudioSample, Message, SampleFormat as SharedSampleFormat};

pub fn get_audio_config() -> Result<AudioConfig> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No default input device"))?;

    let supported_config = device
        .default_input_config()
        .map_err(|e| anyhow::anyhow!("Failed to get default input config: {e}"))?;

    let sample_format = supported_config.sample_format();
    let config: StreamConfig = supported_config.into();

    let shared_sample_format = SharedSampleFormat::try_from(sample_format)?;

    Ok(AudioConfig {
        sample_rate: config.sample_rate.0,
        channels: config.channels,
        sample_format: shared_sample_format,
    })
}

pub fn start_default_input_capture(tx: mpsc::Sender<Message>) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No default input device"))?;

    let supported_config = device
        .default_input_config()
        .map_err(|e| anyhow::anyhow!("Failed to get default input config: {e}"))?;

    let sample_format = supported_config.sample_format();
    let config: StreamConfig = supported_config.into();

    let err_fn = |err| eprintln!("an error occurred on stream: {err}");

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| on_input_data(data, &tx),
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| on_input_data(data, &tx),
            err_fn,
            None,
        )?,
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| on_input_data(data, &tx),
            err_fn,
            None,
        )?,
        SampleFormat::I32 => device.build_input_stream(
            &config,
            move |data: &[i32], _| on_input_data(data, &tx),
            err_fn,
            None,
        )?,
        _ => anyhow::bail!("Unsupported sample format"),
    };

    stream.play()?;
    Ok(stream)
}

fn on_input_data<T: AudioSample>(data: &[T], tx: &mpsc::Sender<Message>) {
    let mut out = Vec::with_capacity(data.len() * T::BYTE_SIZE);
    for &s in data {
        out.extend_from_slice(&s.to_le_bytes().as_ref());
    }
    let _ = tx.send(Message::AudioData(out));
}

