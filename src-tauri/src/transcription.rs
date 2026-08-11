use crate::config::Config;
use hound::{WavSpec, WavWriter};
use std::io::Cursor;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[cfg(windows)]
mod active_window {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    type HWND = *mut std::ffi::c_void;

    extern "system" {
        fn GetForegroundWindow() -> HWND;
        fn GetWindowTextW(hwnd: HWND, lpString: *mut u16, nMaxCount: i32) -> i32;
    }

    pub fn title() -> String {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return String::new();
            }
            let mut buf = [0u16; 256];
            let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if len > 0 {
                OsString::from_wide(&buf[..len as usize])
                    .to_string_lossy()
                    .to_string()
            } else {
                String::new()
            }
        }
    }
}

#[cfg(not(windows))]
mod active_window {
    pub fn title() -> String {
        String::new()
    }
}

fn extract_terms(title: &str) -> Vec<String> {
    let noise = ["gmail", "slack", "discord", "chrome", "firefox", "edge",
        "outlook", "whatsapp", "telegram", "notion", "cursor", "vscode",
        "code", "terminal", "settings", "inbox", "visual studio"];
    let lower = title.to_lowercase();
    if noise.iter().any(|n| lower.contains(n)) && title.split_whitespace().count() <= 3 {
        return vec![];
    }
    title.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .filter(|w| w.chars().next().unwrap_or(' ').is_uppercase())
        .map(|w| w.to_string())
        .collect()
}

fn http_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to create reqwest client")
    })
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

const TARGET_RMS: f32 = 0.05;

fn normalize_gain(samples: &[f32]) -> Vec<f32> {
    let level = rms(samples);
    if level <= 0.0 {
        return samples.to_vec();
    }
    // ponytail: 2.5x cap — an 8x cap amplified background noise along with
    // quiet speech, which is exactly the input Whisper hallucinates on.
    let gain = (TARGET_RMS / level).min(2.5);
    if gain <= 1.0 {
        return samples.to_vec();
    }
    samples.iter().map(|s| (s * gain).clamp(-1.0, 1.0)).collect()
}

fn resample_to_16k(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    const TARGET: u32 = 16000;
    if samples.is_empty() || sample_rate == TARGET {
        return samples.to_vec();
    }
    let ratio = sample_rate as f64 / TARGET as f64;
    let out_len = ((samples.len() - 1) as f64 * TARGET as f64 / sample_rate as f64) as usize + 1;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let a = samples[idx.min(samples.len() - 1)];
        let b = samples[(idx + 1).min(samples.len() - 1)];
        out.push(a + (b - a) * frac);
    }
    out
}

pub fn transcribe(samples: &[f32], sample_rate: u32, config: &Config) -> Result<String, String> {
    if config.api_key.is_empty() {
        return Err("[TXN-001] No API key set. Add your API key in Settings.".into());
    }

    if rms(samples) < 0.0005 {
        println!("TRANSCRIBE: skipping — audio too quiet (rms={:.4})", rms(samples));
        return Ok(String::new());
    }

    // VULN-002: Rate limiting — prevent rapid-fire API calls
    {
        static LAST_CALL: OnceLock<std::sync::Mutex<Instant>> = OnceLock::new();
        let last = LAST_CALL.get_or_init(|| std::sync::Mutex::new(Instant::now() - Duration::from_secs(60)));
        let mut last_time = last.lock().unwrap_or_else(|p| p.into_inner());
        if last_time.elapsed() < Duration::from_millis(1000) {
            return Err("[TXN-005] Recording too fast — wait a second and try again.".into());
        }
        *last_time = Instant::now();
    }

    let samples = normalize_gain(samples);
    let wav_bytes = encode_wav(&samples, sample_rate)?;
    let url = format!("{}/audio/transcriptions", config.api_base_url.trim_end_matches('/'));

    let client = http_client();
    let part = reqwest::blocking::multipart::Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("[TXN-008] Audio encoding failed. ({e})"))?;

    let mut terms: Vec<String> = extract_terms(&active_window::title());
    terms.extend(config.dictionary.iter().cloned());
    terms.extend([
        "gmail", "outlook", "yahoo", "hotmail", "aol",
        "email", "dot com", "dot org", "dot net",
    ].iter().map(|s| s.to_string()));
    terms.sort();
    terms.dedup();
    let prompt = if terms.is_empty() {
        String::new()
    } else {
        format!("Vocabulary hints for this audio: {}.", terms.join(", "))
    };

    let form = reqwest::blocking::multipart::Form::new()
        .part("file", part)
        .text("model", config.model.clone())
        .text("language", "en")
        .text("prompt", prompt);

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .multipart(form)
        .send()
        .map_err(|e| format!("[TXN-002] Network error — check your internet connection. ({e})"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        let code = match status.as_u16() {
            401 => "[TXN-003] Invalid API key (401). Check your key in Settings.",
            404 => "[TXN-004] Model or endpoint not found (404). Check your Model name in Settings.",
            429 => "[TXN-005] Rate limited (429). Wait a moment and try again.",
            s if s >= 500 => "[TXN-006] Server error — try again in a minute.",
            _ => "[TXN-002] API request failed.",
        };
        let trimmed = body.trim();
        let detail = if trimmed.is_empty() { String::new() } else { format!(" ({trimmed})") };
        return Err(format!("{code}{detail}"));
    }

    #[derive(serde::Deserialize)]
    struct WhisperResponse {
        text: String,
    }

    let whisper_resp: WhisperResponse =
        resp.json().map_err(|e| format!("[TXN-007] Could not parse the response. ({e})"))?;

    Ok(whisper_resp.text)
}

fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    if samples.is_empty() {
        return Err("[TXN-008] No audio captured — try speaking closer to the microphone.".into());
    }

    let spec = WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut buf = Cursor::new(Vec::new());
    let mut writer = WavWriter::new(&mut buf, spec).map_err(|e| format!("[TXN-008] Audio encoding failed. ({e})"))?;

    for s in resample_to_16k(samples, sample_rate) {
        let sample = (s * i16::MAX as f32).clamp(-32768.0, 32767.0) as i16;
        writer.write_sample(sample).map_err(|e| format!("[TXN-008] Audio encoding failed. ({e})"))?;
    }

    writer.finalize().map_err(|e| format!("[TXN-008] Audio encoding failed. ({e})"))?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_to_16k_changes_length_and_preserves_sine() {
        let sample_rate = 44100u32;
        let n = 4410;
        let original: Vec<f32> = (0..n)
            .map(|i| {
                (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sample_rate as f32).sin()
            })
            .collect();
        let resampled = resample_to_16k(&original, sample_rate);
        assert_eq!(resampled.len(), 1600);
        let crossings = |v: &[f32]| {
            v.windows(2)
                .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
                .count()
        };
        assert_eq!(crossings(&resampled), crossings(&original));
        assert!((rms(&resampled) - rms(&original)).abs() < rms(&original) * 0.05);
    }

    #[test]
    fn normalize_gain_amplifies_quiet_audio() {
        let quiet: Vec<f32> = (0..4410)
            .map(|i| 0.01 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44100.0).sin())
            .collect();
        let before = rms(&quiet);
        let boosted = normalize_gain(&quiet);
        // rms≈0.007 → gain 7.1, capped at 2.5 → ~2.5x boost
        assert!(rms(&boosted) > before * 2.0, "quiet audio should still be boosted");
        assert!(rms(&boosted) < before * 3.0, "boost must respect the 2.5x cap");
        assert!(boosted.iter().all(|s| s.abs() <= 1.0));
    }
}
