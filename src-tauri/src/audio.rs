use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use nnnoiseless::DenoiseState;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub struct AudioRecorder {
    pub recording: Arc<Mutex<bool>>,
    samples: Arc<Mutex<Vec<f32>>>,
    stream: Option<cpal::Stream>,
    sample_rate: u32,
    channels: u16,
    pub peak_level: Arc<AtomicU32>,
}

// ponytail: cpal::Stream is !Send on Windows (contains raw pointer).
// We ensure stream is stopped/taken before drop, and only access behind a Mutex.
unsafe impl Send for AudioRecorder {}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            recording: Arc::new(Mutex::new(false)),
            samples: Arc::new(Mutex::new(Vec::new())),
            stream: None,
            sample_rate: 0,
            channels: 0,
            peak_level: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn is_recording(&self) -> bool {
        *self.recording.lock().unwrap()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }


    pub fn start(&mut self) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "[REC-001] No microphone found. Plug one in or enable it in Windows settings.".to_string())?;
        let config = device
            .default_input_config()
            .map_err(|e| format!("[REC-002] Could not read microphone settings. ({e})"))?;

        *self.recording.lock().unwrap() = true;
        self.samples.lock().unwrap().clear();
        self.sample_rate = config.sample_rate().0;
        self.channels = config.channels();

        let recording = self.recording.clone();
        let samples = self.samples.clone();
        let peak_level = self.peak_level.clone();

        let err_fn = move |err| println!("audio error: {}", err);

        let stream = device
            .build_input_stream(
                &config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if *recording.lock().unwrap() {
                        samples.lock().unwrap().extend_from_slice(data);
                        let rms = (data.iter().map(|s| s * s).sum::<f32>() / data.len() as f32).sqrt();
                        peak_level.store((rms * 100000.0) as u32, Ordering::Relaxed);
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("[REC-003] Could not start the microphone. ({e})"))?;

        stream.play().map_err(|e| format!("[REC-004] Microphone failed to start. ({e})"))?;
        self.stream = Some(stream);
        Ok(())
    }

    pub fn stop(&mut self) -> Vec<f32> {
        *self.recording.lock().unwrap() = false;
        if let Some(s) = self.stream.take() {
            let _ = s.pause();
        }
        let raw = std::mem::take(&mut *self.samples.lock().unwrap());
        // ponytail: RNNoise is hardcoded to 48kHz internally; at any other sample
        // rate its 480-sample frames aren't 10ms and it corrupts speech, which
        // Whisper hallucinates on. Raw passthrough beats corrupted denoising.
        let samples = if self.sample_rate == 48000 {
            denoise(raw, self.channels as usize)
        } else {
            raw
        };
        trim_silence(&samples, self.sample_rate)
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        // SAFETY: ensures cpal::Stream is stopped before being dropped,
        // upholding the invariant required by `unsafe impl Send`.
        if let Some(stream) = self.stream.take() {
            let _ = stream.pause();
        }
        *self.recording.lock().unwrap_or_else(|p| p.into_inner()) = false;
    }
}

fn denoise(raw: Vec<f32>, channels: usize) -> Vec<f32> {
    if channels == 0 || raw.len() < 480 {
        return raw;
    }

    // Convert stereo to mono by averaging channels
    let mono: Vec<f32> = if channels > 1 {
        raw.chunks_exact(channels)
            .map(|ch| ch.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        raw
    };

    // Process through nnnoiseless in 480-sample frames
    let mut denoiser = DenoiseState::new();
    let mut denoised = Vec::with_capacity(mono.len());
    let mut frame_buf = [0.0f32; 480];

    for chunk in mono.chunks(480) {
        if chunk.len() == 480 {
            denoiser.process_frame(&mut frame_buf, chunk);
            denoised.extend_from_slice(&frame_buf);
        } else {
            // ponytail: skip RNNoise for partial frame to avoid padding artifact
            denoised.extend_from_slice(chunk);
        }
    }

    denoised
}

fn trim_silence(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if sample_rate == 0 || samples.is_empty() {
        return samples.to_vec();
    }

    if samples.len() < sample_rate as usize / 10 {
        return samples.to_vec();
    }

    let frame_size = (sample_rate as f32 * 0.03) as usize;
    if frame_size == 0 {
        return samples.to_vec();
    }

    let threshold = 0.003;
    let margin_frames = 3;

    let voice_frames: Vec<bool> = samples.chunks(frame_size).map(|chunk| {
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        rms > threshold
    }).collect();

    let first = voice_frames.iter().position(|&v| v);
    let last = voice_frames.iter().rposition(|&v| v);

    match (first, last) {
        (Some(f), Some(l)) => {
            let start = f.saturating_sub(margin_frames) * frame_size;
            let end = ((l + 1 + margin_frames) * frame_size).min(samples.len());
            samples[start..end].to_vec()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_idle_recorder() {
        let r = AudioRecorder::new();
        assert!(!r.is_recording());
        assert!(r.stream.is_none());
    }

    #[test]
    fn stop_without_start_returns_empty() {
        let mut r = AudioRecorder::new();
        let samples = r.stop();
        assert!(samples.is_empty());
    }

    #[test]
    fn stop_clears_recording_flag() {
        let mut r = AudioRecorder::new();
        *r.recording.lock().unwrap() = true;
        r.samples.lock().unwrap().push(0.5);
        let samples = r.stop();
        assert!(!r.is_recording());
        assert_eq!(samples, vec![0.5]);
    }

    #[test]
    fn stop_skips_denoise_at_non_48k_sample_rate() {
        let mut r = AudioRecorder::new();
        r.sample_rate = 44100;
        r.channels = 1;
        let raw = vec![0.05f32; 960]; // > 480, would be denoised if the gate were off
        *r.samples.lock().unwrap() = raw.clone();
        let out = r.stop();
        assert_eq!(out, raw, "non-48k audio must bypass RNNoise");
    }

    #[test]
    fn stop_denoises_at_48k() {
        let mut r = AudioRecorder::new();
        r.sample_rate = 48000;
        r.channels = 1;
        let raw = vec![0.05f32; 960]; // constant DC signal, high-passed away by RNNoise
        *r.samples.lock().unwrap() = raw.clone();
        let out = r.stop();
        assert_eq!(out.len(), raw.len());
        assert!(out.iter().any(|s| *s != 0.05), "denoise must alter the signal at 48k");
    }

    #[test]
    fn denoise_mono_passthrough_for_short_audio() {
        // < 480 samples should pass through unchanged
        let input = vec![0.1f32; 100];
        let output = super::denoise(input.clone(), 1);
        assert_eq!(output, input);
    }

    #[test]
    fn denoise_stereo_converts_to_mono() {
        // Stereo (2 channels) with enough samples
        let mut input = Vec::new();
        for _ in 0..480 {
            input.push(0.5); // L
            input.push(0.3); // R
        }
        let output = super::denoise(input, 2);
        // Should produce 480 mono samples (not 960)
        assert_eq!(output.len(), 480);
    }

    #[test]
    fn denoise_survives_empty_input() {
        let output = super::denoise(vec![], 1);
        assert!(output.is_empty());
    }

    #[test]
    fn trim_silence_removes_leading_silence() {
        let sr: u32 = 48000;
        let mut audio = vec![0.0f32; sr as usize]; // 1s silence
        audio.extend(vec![0.1f32; sr as usize]); // 1s voice
        audio.extend(vec![0.0f32; (sr / 2) as usize]); // 0.5s trailing silence
        let trimmed = super::trim_silence(&audio, sr);
        assert!(!trimmed.is_empty(), "voice portion should remain");
        assert!(trimmed.len() < audio.len(), "silence should be trimmed");
    }

    #[test]
    fn trim_silence_returns_empty_for_all_silence() {
        let audio = vec![0.0f32; 48000]; // 1s silence
        let trimmed = super::trim_silence(&audio, 48000u32);
        assert!(trimmed.is_empty());
    }

    #[test]
    fn trim_silence_keeps_short_audio_unchanged() {
        let audio = vec![0.1f32; 100]; // too short (< 100ms)
        let trimmed = super::trim_silence(&audio, 48000u32);
        assert_eq!(trimmed.len(), 100);
    }

    #[test]
    fn start_fails_without_mic() {
        // This runs on any machine - no mic needed to test error handling
        // We can only verify the error type, not that it succeeds
        let mut r = AudioRecorder::new();
        match r.start() {
            Err(msg) => {
                // Expected on machines without mic or if cpal can't init
                assert!(!msg.is_empty());
            }
            Ok(_) => {
                // Has a mic - clean up
                r.stop();
            }
        }
    }
}
