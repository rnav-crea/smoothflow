use crate::config::Config;
use hound::{WavSpec, WavWriter};
use std::io::Cursor;

fn rms(samples: &[f32]) -> f32 {
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

pub fn transcribe(samples: &[f32], sample_rate: u32, config: &Config) -> Result<String, String> {
    if config.api_key.is_empty() {
        return Err("No API key configured. Set it in settings.".into());
    }

    if rms(samples) < 0.0005 {
        println!("TRANSCRIBE: skipping — audio too quiet (rms={:.4})", rms(samples));
        return Ok(String::new());
    }

    let wav_bytes = encode_wav(samples, sample_rate)?;
    let url = format!("{}/audio/transcriptions", config.api_base_url.trim_end_matches('/'));

    let client = reqwest::blocking::Client::new();
    let part = reqwest::blocking::multipart::Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;

    let form = reqwest::blocking::multipart::Form::new()
        .part("file", part)
        .text("model", config.model.clone())
        .text("language", "en");

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .multipart(form)
        .send()
        .map_err(|e| format!("API request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("API error {}: {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct WhisperResponse {
        text: String,
    }

    let whisper_resp: WhisperResponse =
        resp.json().map_err(|e| format!("failed to parse response: {}", e))?;

    Ok(whisper_resp.text)
}

fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    // Downsample to 16kHz (Whisper API expects 16kHz mono)
    let target_rate = 16000u32;
    let step = (sample_rate / target_rate) as usize;

    let spec = WavSpec {
        channels: 1,
        sample_rate: target_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut buf = Cursor::new(Vec::new());
    let mut writer = WavWriter::new(&mut buf, spec).map_err(|e| e.to_string())?;

    for &s in samples.iter().step_by(step) {
        let sample = (s * i16::MAX as f32).clamp(-32768.0, 32767.0) as i16;
        writer.write_sample(sample).map_err(|e| e.to_string())?;
    }

    writer.finalize().map_err(|e| e.to_string())?;
    Ok(buf.into_inner())
}
