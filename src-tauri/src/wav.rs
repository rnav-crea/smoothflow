//! Minimal 16-bit mono WAV writer — `std` only.
//!
//! Dictation clips are plain PCM WAV, like Handy (cjpais/Handy): no
//! compression, no container crate. `hound` is already a dependency for the
//! cloud upload encoding, but the clip writer deliberately keeps its own 44-byte
//! header so a dependency bump can never change the file format on disk.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Canonical RIFF/WAVE PCM header length.
const HEADER_LEN: u32 = 44;

/// Write `samples` (mono, `f32` in `[-1.0, 1.0]`) as a 16-bit PCM WAV file.
///
/// Out-of-range samples are clamped rather than wrapped, so a hot microphone
/// cannot turn a loud word into noise.
pub fn write_wav_16bit_mono(path: &Path, samples: &[f32], sample_rate: u32) -> std::io::Result<()> {
    let data_len = (samples.len() as u32).saturating_mul(2);
    let byte_rate = sample_rate.saturating_mul(2);
    let mut out = BufWriter::new(File::create(path)?);

    // RIFF container: "RIFF" <riff_size> "WAVE". riff_size counts everything
    // after the first 8 bytes, i.e. the 36-byte remainder of the header plus
    // the data chunk.
    out.write_all(b"RIFF")?;
    out.write_all(&(HEADER_LEN - 8 + data_len).to_le_bytes())?;
    out.write_all(b"WAVE")?;

    // fmt chunk: 16 bytes of PCM description.
    out.write_all(b"fmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?; // audio_format = 1 (PCM)
    out.write_all(&1u16.to_le_bytes())?; // channels = 1
    out.write_all(&sample_rate.to_le_bytes())?;
    out.write_all(&byte_rate.to_le_bytes())?;
    out.write_all(&2u16.to_le_bytes())?; // block_align = 2
    out.write_all(&16u16.to_le_bytes())?; // bits_per_sample

    // data chunk.
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;

    // ponytail: one buffer, one write. A 10-minute clip is ~57 MB, which a
    // BufWriter would split anyway; this keeps the header and samples adjacent.
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    out.write_all(&pcm)?;
    out.flush()
}

/// Errors a clip read can produce. Kept as `String` to match the rest of the
/// crate's fallible helpers — this is a log-and-tell-the-user path, not a
/// typed API boundary.
fn bad_clip(msg: &str) -> String {
    format!("[WAV-001] {msg}")
}

/// Read a 16-bit mono PCM WAV back into `(samples, sample_rate)`.
///
/// Samples are `f32` in `[-1.0, 1.0]`. This is the inverse of
/// [`write_wav_16bit_mono`], used to re-dictate a saved clip without the user
/// having to speak again — the sample rate comes back with the samples because
/// the transcription engine needs it to interpret them.
///
/// Chunk-walking rather than assuming a fixed 44-byte header: real-world WAVs
/// (and our own after any future writer change) can carry extra chunks before
/// `data`, and a reader that assumes an offset silently produces noise.
///
/// Anything that is not 16-bit mono PCM is rejected loudly rather than
/// half-decoded — a stereo or 24-bit file would otherwise produce garbage
/// transcript text.
pub fn read_wav_16bit_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let bytes = std::fs::read(path).map_err(|e| bad_clip(&format!("could not read {}: {e}", path.display())))?;
    if bytes.len() < 44 {
        return Err(bad_clip("file is too short to be a WAV"));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad_clip("missing RIFF/WAVE signature"));
    }

    // Walk the chunk list from byte 12. Chunks are "<4-byte id><u32 LE size>"
    // followed by `size` bytes, padded to an even boundary.
    let mut pos = 12usize;
    let mut fmt = None;
    let mut data = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " => fmt = Some(body),
            b"data" => {
                data = Some(body);
                break;
            }
            _ => {}
        }
        // Chunks are word-aligned: an odd `size` is followed by a pad byte.
        pos = body_start + size + (size & 1);
    }

    let fmt = fmt.ok_or_else(|| bad_clip("no fmt chunk"))?;
    if fmt.len() < 16 {
        return Err(bad_clip("fmt chunk is truncated"));
    }
    let audio_format = u16::from_le_bytes(fmt[0..2].try_into().unwrap());
    let channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap());
    let bits = u16::from_le_bytes(fmt[14..16].try_into().unwrap());
    if audio_format != 1 {
        return Err(bad_clip(&format!("expected PCM, got format tag {audio_format}")));
    }
    if channels != 1 {
        return Err(bad_clip(&format!("expected mono, got {channels} channels")));
    }
    if bits != 16 {
        return Err(bad_clip(&format!("expected 16-bit, got {bits}-bit")));
    }

    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
    if sample_rate == 0 {
        return Err(bad_clip("sample rate is zero"));
    }

    let data = data.ok_or_else(|| bad_clip("no data chunk"))?;
    let mut out = Vec::with_capacity(data.len() / 2);
    for sample in data.chunks_exact(2) {
        // 32767, not 32768: the exact inverse of the writer's `* 32767.0`, so a
        // clip round-trips to within half a quantisation step instead of
        // carrying a permanent full-scale offset.
        out.push(i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32767.0);
    }
    Ok((out, sample_rate))
}

/// Errors a *user-picked* file read can produce. Separate code from
/// `[WAV-001]` so a log line says which of the two readers failed.
fn bad_file(msg: &str) -> String {
    format!("[WAV-002] {msg}")
}

/// Read any WAV `hound` can open as mono `f32` at its native sample rate.
///
/// Used by "Transcribe file": a user's WAV can be 8/16/24/32-bit, float, and
/// stereo, and rejecting those would make the feature useless. The clip
/// reader above stays strict because a saved clip is always our own format.
///
/// Multi-channel audio is averaged down to mono per frame — summing would
/// clip on loud stereo material. Integer samples are scaled by `2^(bits-1)`,
/// so 8-bit unsigned WAVs (which `hound` already re-centres on zero) and
/// 16/24/32-bit signed WAVs all land in `[-1.0, 1.0]`.
///
/// Capped at 30 minutes: a stray multi-hour recording would otherwise
/// allocate gigabytes of `f32` before anything could reject it.
pub fn read_wav_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let reader = hound::WavReader::open(path)
        .map_err(|e| bad_file(&format!("could not open {}: {e}", path.display())))?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let sample_rate = spec.sample_rate;
    let bits = spec.bits_per_sample;

    if channels == 0 {
        return Err(bad_file(&format!(
            "{} declares zero channels",
            path.display()
        )));
    }
    if sample_rate == 0 {
        return Err(bad_file(&format!(
            "{} declares a zero sample rate",
            path.display()
        )));
    }
    if bits == 0 {
        return Err(bad_file(&format!(
            "{} declares zero bits per sample",
            path.display()
        )));
    }

    // 30 minutes at the file's own rate — the limit is duration, not bytes, so
    // a 96 kHz file is not treated as 3× longer than a 32 kHz one. Checked
    // from the header *before* decoding: the point of the cap is to never
    // allocate the gigabytes a stray 3-hour recording would need.
    let max_samples = sample_rate as u64 * 1800;
    if reader.duration() as u64 > max_samples {
        return Err(bad_file(&format!(
            "{} is longer than the 30-minute limit for transcription",
            path.display()
        )));
    }

    // Interleaved, still one value per channel.
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            // 32768 for 16-bit, not 32767: the symmetric denominator keeps a
            // full-scale negative sample at exactly -1.0 instead of drifting
            // below the range every other sample is measured against.
            let scale = 2f32.powi(bits as i32 - 1);
            let mut out = Vec::new();
            for sample in reader.into_samples::<i32>() {
                let v = sample
                    .map_err(|e| bad_file(&format!("could not decode {}: {e}", path.display())))?;
                out.push(v as f32 / scale);
            }
            out
        }
        hound::SampleFormat::Float => {
            let mut out = Vec::new();
            for sample in reader.into_samples::<f32>() {
                let v = sample
                    .map_err(|e| bad_file(&format!("could not decode {}: {e}", path.display())))?;
                out.push(v);
            }
            out
        }
    };

    let mut samples = Vec::with_capacity(interleaved.len() / channels);
    for frame in interleaved.chunks(channels) {
        let sum: f32 = frame.iter().sum();
        samples.push(sum / frame.len() as f32);
    }

    if samples.is_empty() {
        return Err(bad_file(&format!("{} contains no audio", path.display())));
    }
    // Belt and braces: a header that understated its own length cannot get past
    // this one either.
    if samples.len() as u64 > max_samples {
        return Err(bad_file(&format!(
            "{} is longer than the 30-minute limit for transcription",
            path.display()
        )));
    }

    Ok((samples, sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-verified only: `cargo test` cannot load its binary on this
    /// Windows box, CI is the only place these actually run.
    fn header_bytes(samples: &[f32], sample_rate: u32) -> Vec<u8> {
        let path = std::env::temp_dir().join(format!(
            "sf-wav-test-{}-{}.wav",
            std::process::id(),
            samples.len()
        ));
        write_wav_16bit_mono(&path, samples, sample_rate).expect("write wav");
        let bytes = std::fs::read(&path).expect("read wav");
        let _ = std::fs::remove_file(&path);
        bytes
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    fn u16_at(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
    }

    #[test]
    fn zero_samples_writes_a_bare_header() {
        let b = header_bytes(&[], 16_000);
        assert_eq!(b.len(), 44);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32_at(&b, 4), 36);
        assert_eq!(&b[8..12], b"WAVE");
        assert_eq!(u32_at(&b, 16), 16);
        assert_eq!(u16_at(&b, 20), 1);
        assert_eq!(u16_at(&b, 22), 1);
        assert_eq!(u32_at(&b, 24), 16_000);
        assert_eq!(u32_at(&b, 28), 32_000);
        assert_eq!(u16_at(&b, 32), 2);
        assert_eq!(u16_at(&b, 34), 16);
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32_at(&b, 40), 0);
    }

    #[test]
    fn three_samples_are_clamped_and_little_endian() {
        let b = header_bytes(&[0.0, 1.0, -2.0], 48_000);
        assert_eq!(b.len(), 50);
        assert_eq!(u32_at(&b, 4), 42);
        assert_eq!(u32_at(&b, 40), 6);
        assert_eq!(i16::from_le_bytes([b[44], b[45]]), 0);
        assert_eq!(i16::from_le_bytes([b[46], b[47]]), 32_767);
        assert_eq!(i16::from_le_bytes([b[48], b[49]]), -32_767);
    }

    /// Round-trip: what we write is what we read back. The exact float values
    /// differ by quantisation, so this asserts a tight bound rather than equality.
    #[test]
    fn a_written_clip_reads_back_unchanged_within_quantisation() {
        let samples = vec![0.0, 0.5, -0.5, 1.0, -1.0, 0.123_456];
        let path = std::env::temp_dir().join(format!(
            "sf-wav-roundtrip-{}.wav",
            std::process::id()
        ));
        write_wav_16bit_mono(&path, &samples, 16_000).expect("write wav");
        let (back, rate) = read_wav_16bit_mono(&path).expect("read wav");
        let _ = std::fs::remove_file(&path);

        assert_eq!(rate, 16_000);
        assert_eq!(back.len(), samples.len());
        for (a, b) in samples.iter().zip(back.iter()) {
            assert!(
                (a - b).abs() < 1.0 / 32000.0,
                "round-trip drifted: {a} vs {b}"
            );
        }
    }

    #[test]
    fn an_empty_clip_reads_back_as_no_samples() {
        let path = std::env::temp_dir().join(format!("sf-wav-empty-{}.wav", std::process::id()));
        write_wav_16bit_mono(&path, &[], 16_000).expect("write wav");
        let (back, rate) = read_wav_16bit_mono(&path).expect("read wav");
        let _ = std::fs::remove_file(&path);
        assert!(back.is_empty());
        assert_eq!(rate, 16_000);
    }

    #[test]
    fn a_stereo_or_24_bit_file_is_rejected_not_half_decoded() {
        let path = std::env::temp_dir().join(format!("sf-wav-fake-{}.wav", std::process::id()));
        let mut b = header_bytes(&[0.0; 8], 16_000);
        // Claim 2 channels where the writer said 1.
        b[22] = 2;
        std::fs::write(&path, &b).expect("write fake");
        let err = read_wav_16bit_mono(&path).expect_err("stereo must be rejected");
        let _ = std::fs::remove_file(&path);
        assert!(err.contains("mono"), "unexpected error: {err}");
    }

    #[test]
    fn a_truncated_or_non_wav_file_is_rejected() {
        let path = std::env::temp_dir().join(format!("sf-wav-junk-{}.wav", std::process::id()));
        std::fs::write(&path, b"not a wav at all").expect("write junk");
        let err = read_wav_16bit_mono(&path).expect_err("junk must be rejected");
        let _ = std::fs::remove_file(&path);
        assert!(err.contains("WAV") || err.contains("RIFF"), "unexpected: {err}");
    }

    /// A chunk list with an odd-sized chunk before `data` must still find the
    /// samples — the word-alignment pad is the classic way a naive reader
    /// slips out of sync.
    #[test]
    fn an_extra_odd_sized_chunk_still_finds_the_data() {
        let samples = [0.25f32, -0.25];
        let path = std::env::temp_dir().join(format!("sf-wav-extra-{}.wav", std::process::id()));
        write_wav_16bit_mono(&path, &samples, 16_000).expect("write wav");
        let b = std::fs::read(&path).expect("read wav");

        // Splice a 3-byte "LIST" chunk in before "data", with its pad byte.
        let mut spliced = Vec::new();
        spliced.extend_from_slice(&b[0..4]);
        let extra: &[u8] = b"LIST";
        let mut body = extra.to_vec();
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(b"odd");
        body.push(0); // alignment pad
        let new_data_len = b.len() as u32 - 12 + body.len() as u32;
        spliced.extend_from_slice(&new_data_len.to_le_bytes());
        spliced.extend_from_slice(&b[8..12]);
        spliced.extend_from_slice(&b[12..36]); // the original fmt chunk
        spliced.extend_from_slice(&body);
        spliced.extend_from_slice(&b[36..]); // data chunk onward
        // RIFF size counts everything after byte 8.
        let riff_size = (spliced.len() - 8) as u32;
        spliced[4..8].copy_from_slice(&riff_size.to_le_bytes());
        std::fs::write(&path, &spliced).expect("write spliced");

        let (back, rate) = read_wav_16bit_mono(&path).expect("read spliced");
        let _ = std::fs::remove_file(&path);
        assert_eq!(rate, 16_000);
        assert_eq!(back.len(), 2);
        assert!((back[0] - 0.25).abs() < 0.001);
        assert!((back[1] + 0.25).abs() < 0.001);
    }

    /// The permissive reader: a stereo file the strict reader rejects must come
    /// back as the per-frame average, at the file's own rate.
    #[test]
    fn a_stereo_file_is_mixed_down_to_mono_by_averaging() {
        let path = std::env::temp_dir().join(format!("sf-wav-mono-{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
            // Opposing channels cancel; matching channels survive.
            w.write_sample(16_384i16).expect("l");
            w.write_sample(-16_384i16).expect("r");
            w.write_sample(8_192i16).expect("l");
            w.write_sample(8_192i16).expect("r");
            w.finalize().expect("finalize");
        }

        let (samples, rate) = read_wav_mono(&path).expect("read stereo");
        let _ = std::fs::remove_file(&path);

        assert_eq!(rate, 44_100);
        assert_eq!(samples.len(), 2);
        assert!(samples[0].abs() < 0.001, "opposing channels: {}", samples[0]);
        assert!((samples[1] - 0.25).abs() < 0.001, "matching channels: {}", samples[1]);
    }
}
