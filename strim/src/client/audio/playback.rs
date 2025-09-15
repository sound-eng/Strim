use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;

use strim_shared::{Message, AudioConfig, SampleFormat as SharedSampleFormat, AudioSample};

pub fn start_audio_playback(rx: mpsc::Receiver<Message>, running: Arc<AtomicBool>) -> Result<()> {
    let audio_config = match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Message::Config(config)) => config,
        Ok(Message::Error(err)) => return Err(anyhow::anyhow!("Server error: {}", err)),
        Ok(Message::AudioData(_)) => AudioConfig { sample_rate: 44100, channels: 2, sample_format: SharedSampleFormat::F32 },
        Ok(Message::Ping) => return Err(anyhow::anyhow!("Expected config on playback start, got Ping")),
        Err(e) => return Err(anyhow::anyhow!("Failed to receive config: {}", e)),
    };

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No default output device"))?;

    let config = StreamConfig {
        channels: audio_config.channels,
        sample_rate: cpal::SampleRate(audio_config.sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let sample_format: SampleFormat = audio_config.sample_format.into();

    let err_fn = |err| eprintln!("Audio playback error: {err}");
    let audio_buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let buffer_for_network = Arc::clone(&audio_buffer);
    let buffer_for_audio = Arc::clone(&audio_buffer);

    let running_audio = Arc::clone(&running);
    thread::spawn(move || {
        while running_audio.load(Ordering::SeqCst) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(message) => match message {
                    Message::AudioData(data) => {
                        let mut buffer = buffer_for_network.lock().unwrap();
                        buffer.extend_from_slice(&data);
                    }
                    Message::Config(_) => {}
                    Message::Ping => {}
                    Message::Error(err) => eprintln!("Server error: {}", err),
                },
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        }
    });

    let shared_sample_format = SharedSampleFormat::try_from(sample_format)?;

    let stream = match shared_sample_format {
        SharedSampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| on_output_data(data, &buffer_for_audio),
            err_fn,
            None,
        )?,
        SharedSampleFormat::I16 => device.build_output_stream(
            &config,
            move |data: &mut [i16], _| on_output_data(data, &buffer_for_audio),
            err_fn,
            None,
        )?,
        SharedSampleFormat::U16 => device.build_output_stream(
            &config,
            move |data: &mut [u16], _| on_output_data(data, &buffer_for_audio),
            err_fn,
            None,
        )?,
        SharedSampleFormat::I32 => device.build_output_stream(
            &config,
            move |data: &mut [i32], _| on_output_data(data, &buffer_for_audio),
            err_fn,
            None,
        )?,
        // No other variants currently
    };

    stream.play()?;
    while running.load(Ordering::SeqCst) { thread::sleep(Duration::from_millis(100)); }
    drop(stream);
    Ok(())
}

fn on_output_data<T: AudioSample>(data: &mut [T], buffer: &Arc<Mutex<Vec<u8>>>) {
    let mut audio_buffer = buffer.lock().unwrap();
    let bytes_needed = data.len() * T::BYTE_SIZE;
    if audio_buffer.len() < bytes_needed { data.fill(T::default()); return; }
    for (i, sample_bytes) in audio_buffer[..bytes_needed].chunks(T::BYTE_SIZE).enumerate() {
        data[i] = T::from_le_bytes(sample_bytes);
    }
    audio_buffer.drain(..bytes_needed);
}

