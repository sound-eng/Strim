use core::time;
use anyhow::Result;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Device};

use ringbuf::{traits::*, HeapRb, producer::Producer, consumer::Consumer};

pub struct Capture {
    ringbuffer: HeapRb<f32>,
    // stream: cpal::Stream,
}

impl Capture {

    pub fn device(device_id: &String) -> Option<Device> {
        let host = cpal::default_host();
        let mut capture_devices = host.input_devices().ok()?;
        let wanted = device_id;
        let found = capture_devices.find(|d| d.name().map_or(false, |n| n == *wanted));
        found
    }   

    pub fn capture_devices() -> Vec<Device> {
        let host = cpal::default_host();
        let mut res = host.input_devices();
        match res {
            Ok(iter) => iter.collect(),
            Err(e) => Vec::new()
        }
    }

    pub fn new() -> Self {
        let host = cpal::default_host();

        let device = host
            .default_input_device()
            .expect("Failed to get default input device");

        let config = device
            .default_input_config()
            .expect("Failed to get default input config");

        let sample_format = config.sample_format();
        let config: cpal::StreamConfig = config.into();

        let ringbuffer = HeapRb::<f32>::new(48000 * 10); // 10 seconds buffer at 48kHz

        Self {
            ringbuffer,
            // stream,
        }
    }

    /// Function that starts capture
    /// 
    pub fn start(&self) -> Result<()> {
        let err_fn = |err| eprintln!("an error occurred on stream: {}", err);
        let data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for &sample in data {
                self.ringbuffer.push_overwrite(sample);
            }
        };

        let (producer, consumer) = self.ringbuffer.split();

        let sample_format = config.sample_format();
        let config: cpal::StreamConfig = config.into();

        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(&config, data_fn, err_fn, None),
            _ => panic!("Unsupported sample format"),
        }
        .expect("Failed to build input stream");

        stream.play().expect("Failed to play stream");
        Ok(())
    }

}

// a test for ringbuffer functionality where we create a ringbuffer, write to it, and read from it ringbuffer of size 1024,
// write 512 samples, read 256 samples, write another 512 samples, read 768 samples, and ensure the data is correct.
// we also ensure that the ringbuffer does not overflow and that the data is not lost.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_ringbuffer() {
        let ringbuffer = HeapRb::<f32>::new(1024);
        let (mut producer, mut consumer) = ringbuffer.split();

        // Write 512 samples
        for i in 0..512 {
            producer.try_push(i as f32).unwrap();
        }

        // Read 256 samples
        let mut read_samples = Vec::new();
        for _ in 0..256 {
            read_samples.push(consumer.try_pop().unwrap());
        }

        // Write another 512 samples
        for i in 512..1024 {
            producer.try_push(i as f32).unwrap();
        }

        // Read 768 samples
        let mut read_samples_2 = Vec::new();
        for _ in 0..768 {
            read_samples_2.push(consumer.try_pop().unwrap());
        }

        // Ensure the data is correct
        assert_eq!(read_samples, (0..256).map(|x| x as f32).collect::<Vec<_>>());
        assert_eq!(read_samples_2, (256..1024).map(|x| x as f32).collect::<Vec<_>>());
    }   

    // test ringbuffer without using producer and consumer split, just call the appropriate functions on the ringbuffer directly.
    #[test]
    fn test_ringbuffer_direct() {
        let mut ringbuffer = HeapRb::<f32>::new(1024);

        // Create array of 512 samples:
        let mut samples = [0f32; 512];
        for i in 0..512 {
            samples[i] = i as f32;
        }

        ringbuffer.push_slice(&samples);

        // Read 256 samples
        let mut read_samples = Vec::new();
        for _ in 0..256 {
            read_samples.push(ringbuffer.try_pop().unwrap());
        }

        // Write another 512 samples
        for i in 512..1024 {
            ringbuffer.try_push(i as f32).unwrap();
        }

        // Read 768 samples
        let mut read_samples_2 = Vec::new();
        for _ in 0..768 {
            read_samples_2.push(ringbuffer.try_pop().unwrap());
        }

        // Ensure the data is correct
        assert_eq!(read_samples, (0..256).map(|x| x as f32).collect::<Vec<_>>());
        assert_eq!(read_samples_2, (256..1024).map(|x| x as f32).collect::<Vec<_>>());
    }   
}