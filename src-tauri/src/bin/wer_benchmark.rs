fn word_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let ref_words: Vec<&str> = reference.split_whitespace().collect();
    let hyp_words: Vec<&str> = hypothesis.split_whitespace().collect();
    let m = ref_words.len();
    let n = hyp_words.len();
    if m == 0 {
        return if n == 0 { 0.0 } else { 1.0 };
    }
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m {
        dp[i][0] = i;
    }
    for j in 0..=n {
        dp[0][j] = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            let cost = if ref_words[i - 1] == hyp_words[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1).min(dp[i][j - 1] + 1).min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[m][n] as f64 / m as f64
}

fn main() {
    let test_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test_data");
    if !test_dir.exists() {
        eprintln!();
        eprintln!("========================================================================");
        eprintln!(" No test_data/ folder found.");
        eprintln!("========================================================================");
        eprintln!(" To run the accuracy benchmark:");
        eprintln!(" 1. Create folder: src-tauri/test_data/");
        eprintln!(" 2. Add .wav files (16-bit mono, any sample rate)");
        eprintln!(" 3. For each .wav, add a .txt file with the exact same name");
        eprintln!("    containing the ground-truth transcript (what was actually said)");
        eprintln!(" 4. Make sure your API key is set (run SmoothFlow once, set it in Settings)");
        eprintln!(" 5. Run: cd src-tauri && cargo run --bin wer_benchmark");
        eprintln!("========================================================================");
        eprintln!();
        return;
    }

    let config = smoothflow_lib::config::Config::load();
    if config.api_key.is_empty() {
        eprintln!("ERROR: No API key found. Run SmoothFlow and set your key in Settings first.");
        std::process::exit(1);
    }

    let mut entries: Vec<_> = std::fs::read_dir(&test_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|s| s == "wav").unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.path());

    if entries.is_empty() {
        eprintln!("No .wav files found in test_data/");
        return;
    }

    let mut total_wer = 0.0;
    let mut count = 0;

    println!();
    println!("{:=<68}", "");
    println!(" ACCURACY BENCHMARK (Word Error Rate)");
    println!("{:=<68}", "");
    println!("{:30} {:>8} {:>8} {:>12}", "File", "WER%", "Accuracy%", "Latency");
    println!("{:-<68}", "");

    for entry in &entries {
        let wav_path = entry.path();
        let txt_path = wav_path.with_extension("txt");

        let ground_truth = std::fs::read_to_string(&txt_path)
            .unwrap_or_else(|_| panic!("Missing ground-truth file: {:?}", txt_path))
            .trim()
            .to_lowercase();

        let mut reader = hound::WavReader::open(&wav_path)
            .unwrap_or_else(|e| panic!("Cannot read {:?}: {}", wav_path, e));
        let spec = reader.spec();
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / i16::MAX as f32)
            .collect();

        let start = std::time::Instant::now();
        let result = smoothflow_lib::transcription::transcribe(&samples, spec.sample_rate, &config)
            .unwrap_or_default();
        let elapsed = start.elapsed();

        let result_clean = result.to_lowercase().trim().to_string();
        let wer = word_error_rate(&ground_truth, &result_clean);
        let accuracy = (1.0 - wer) * 100.0;
        total_wer += wer;
        count += 1;

        println!(
            "{:30} {:>7.1}% {:>7.1}% {:>5}s",
            entry.file_name().to_string_lossy(),
            wer * 100.0,
            accuracy,
            elapsed.as_secs()
        );

        if wer > 0.3 {
            println!("  └─ REF:  {}", ground_truth);
            println!("  └─ HYP:  {}", result_clean);
        }
    }

    if count > 0 {
        let avg_wer = total_wer / count as f64;
        let avg_acc = (1.0 - avg_wer) * 100.0;
        println!("{:-<68}", "");
        println!("{:30} {:>7.1}% {:>7.1}%", "AVERAGE", avg_wer * 100.0, avg_acc);
        println!("{:=<68}", "");
    }
}
