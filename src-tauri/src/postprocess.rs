use crate::config::Config;
use std::sync::OnceLock;
use std::time::Duration;

fn http_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to create reqwest client")
    })
}

const FILLERS: &[&str] = &[
    "um", "uh", "like", "you know", "actually", "basically",
    "literally", "sort of", "kind of", "i mean", "well",
];

const ENHANCE_SYSTEM_PROMPT: &str = "You clean up raw dictation transcripts.\n\n\
CRITICAL RULE: Output EXACTLY the same number of lines as the input.\n\
Do not add, remove, merge, or split lines. Each input line becomes one output line.\n\n\
Rules:\n\
1. RESOLVE SELF-CORRECTIONS: keep only the final intent.\n\
   \"I will meet at 6pm no wait 7pm\" -> \"I will meet at 7pm\"\n\
   \"today no tomorrow\" -> \"tomorrow\"\n\n\
2. Fix obvious grammar and spelling only. Keep specific details unchanged.\n\
   \"i hitted the bottle\" -> \"I hit the bottle\"\n\
   \"water fell on the divice\" -> \"water fell on the device\"\n\n\
3. EMAIL ADDRESSES: convert spoken \"at\" to @ when clearly an email.\n\
   \"navin at redmail.com\" -> \"navin@redmail.com\"\n\n\
4. Add basic punctuation (periods at sentence end).\n\n\
5. Preserve the original perspective exactly as dictated — do not convert between first and third person.\n\n\
6. Output ONLY the cleaned transcript lines. No explanations, no notes.";

#[derive(Debug, PartialEq)]
enum ResolveOutcome {
    NoCorrection,
    Resolved(String),
    Ambiguous,
}

// "A instead of B" / "A rather than B": the first option is the final intent.
const FIRST_WINS_MARKERS: &[&str] = &["instead of", "rather than"];

// "A no wait B": the last option is the final intent. Longest-first so the
// alternation prefers "no wait" over "no" at the same position.
const LAST_WINS_MARKERS: &[&str] = &[
    "no, wait", "no wait", "not that", "not this",
    "scratch that", "forget that", "i meant", "i mean", "actually", "no", "wait",
];

pub fn postprocess(text: &str, config: &Config) -> String {
    if config.cleanup_model.is_empty() {
        return basic_cleanup(text, config);
    }
    match resolve_self_corrections(text) {
        // resolved corrections still get the finishing pass (fillers, emails,
        // dictionary, punctuation) — just not another resolution attempt
        ResolveOutcome::Resolved(fixed) => basic_cleanup(&fixed, config),
        ResolveOutcome::NoCorrection => basic_cleanup(text, config),
        ResolveOutcome::Ambiguous => match cleanup_transcript(text, config) {
            Ok(cleaned) if !cleaned.trim().is_empty() => basic_cleanup(&cleaned, config),
            // LLM is optional: silent fallback so dictation never breaks on service failure
            _ => basic_cleanup(text, config),
        },
    }
}

fn resolve_self_corrections(text: &str) -> ResolveOutcome {
    static FIRST_RE: OnceLock<regex::Regex> = OnceLock::new();
    static LAST_RE: OnceLock<regex::Regex> = OnceLock::new();
    let first_re = FIRST_RE.get_or_init(|| marker_re(FIRST_WINS_MARKERS));
    let last_re = LAST_RE.get_or_init(|| marker_re(LAST_WINS_MARKERS));
    let (last_wins, m) = match (first_re.find(text), last_re.find(text)) {
        (Some(fm), Some(lm)) if fm.start() <= lm.start() => (false, fm),
        (Some(_), Some(lm)) => (true, lm),
        (Some(fm), None) => (false, fm),
        (None, Some(lm)) => (true, lm),
        (None, None) => return ResolveOutcome::NoCorrection,
    };
    let before = text[..m.start()].trim();
    let after = text[m.end()..].trim();
    // A leading marker phrase ("no wait I meant 4 pm") belongs to the SAME
    // correction — strip it first so it isn't mistaken for a second one.
    let tail = strip_leading_markers(after);
    // conservative: a correction must split a real utterance, both sides non-trivial
    if before.is_empty() || !tail.chars().any(|c| c.is_alphanumeric()) {
        return ResolveOutcome::Ambiguous;
    }
    // a leftover marker in the tail is a correction chain -> not confidently resolvable
    if first_re.find(&tail).is_some() || last_re.find(&tail).is_some() {
        return ResolveOutcome::Ambiguous;
    }
    let resolved = if last_wins {
        let mut words: Vec<&str> = before.split_whitespace().collect();
        // the superseded value: drop from the last number token ("3 PM", "6pm");
        // if the corrected thing was a plain word ("today", "the park"), drop the last word
        match words.iter().rposition(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit())) {
            Some(i) => words.truncate(i),
            None => { words.pop(); }
        }
        let head = words.join(" ");
        let combined = if head.is_empty() { tail } else { format!("{head} {tail}") };
        collapse_adjacent_dupes(&combined)
    } else {
        // "meet at 5pm instead of 4pm": keep the first option, drop the rest
        before.to_string()
    };
    // ponytail: heuristic value-drop + adjacent-dupe collapse; phrase boundaries,
    // correction chains, and non-correctional "no/wait/actually" aren't distinguished
    ResolveOutcome::Resolved(resolved)
}

fn strip_leading_markers(text: &str) -> String {
    static LEAD_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = LEAD_RE.get_or_init(|| {
        let phrases = [
            "i mean", "i meant", "no wait", "no, wait", "oh wait",
            "scratch that", "forget that", "not that", "not this",
            "sorry", "wait", "actually", "no", "oh",
        ];
        let alts: Vec<String> = phrases.iter().map(|p| regex::escape(p)).collect();
        regex::Regex::new(&format!(r"(?i)^\s*(?:{})\s*[\s,]+", alts.join("|"))).unwrap()
    });
    let mut s = text.to_string();
    loop {
        let next = re.replace(&s, "").into_owned();
        if next == s {
            break;
        }
        s = next;
    }
    s
}

fn marker_re(markers: &[&str]) -> regex::Regex {
    let alts: Vec<String> = markers.iter().map(|m| regex::escape(m)).collect();
    regex::Regex::new(&format!(r"(?i)\b(?:{})\b", alts.join("|"))).unwrap()
}

fn collapse_adjacent_dupes(s: &str) -> String {
    let mut prev: Option<&str> = None;
    let mut out: Vec<&str> = Vec::new();
    for w in s.split_whitespace() {
        if !prev.is_some_and(|p| p.eq_ignore_ascii_case(w)) {
            out.push(w);
        }
        prev = Some(w);
    }
    out.join(" ")
}

fn remove_hallucination_loops(text: &str) -> String {
    // ponytail: whitespace-token runs; collapses a token repeated 3+ times
    // consecutively to one. Won't catch 2-token loops ("thank you thank you")
    // or non-adjacent repeats — those need the LLM path, not a regex.
    let mut runs: Vec<Vec<&str>> = Vec::new();
    for w in text.split_whitespace() {
        match runs.last_mut() {
            Some(last) if last.last().is_some_and(|p| p.eq_ignore_ascii_case(w)) => last.push(w),
            _ => runs.push(vec![w]),
        }
    }
    let mut out: Vec<&str> = Vec::new();
    for run in runs {
        if run.len() >= 3 {
            out.push(run[0]);
        } else {
            out.extend(run);
        }
    }
    out.join(" ")
}

fn basic_cleanup(text: &str, config: &Config) -> String {
    let text = remove_hallucination_loops(text);

    let text = if config.remove_fillers {
        remove_fillers(&text)
    } else {
        text
    };

    let text = convert_spoken_emails(&text);

    let text = correct_dictionary_terms(&text, &config.dictionary);

    if config.auto_punctuation {
        add_punctuation(&text)
    } else {
        text
    }
}

fn convert_spoken_emails(text: &str) -> String {
    static RE_MULTI: OnceLock<regex::Regex> = OnceLock::new();
    static RE_MULTI2: OnceLock<regex::Regex> = OnceLock::new();
    static RE_DOT: OnceLock<regex::Regex> = OnceLock::new();
    static RE_DOT2: OnceLock<regex::Regex> = OnceLock::new();
    static RE_NO_TLD: OnceLock<regex::Regex> = OnceLock::new();

    // Step 1: fix multi-level TLDs first ("university dot edu dot in" → "university.edu.in")
    let re_multi = RE_MULTI.get_or_init(|| regex::Regex::new(
        r"(?i)(\w+)\s+dot\s+(\w+)\s+dot\s+(\w+)"
    ).unwrap());
    let text = re_multi.replace_all(&text, |caps: &regex::Captures| {
        format!("{}.{}.{}", &caps[1], &caps[2], &caps[3])
    }).to_string();

    // "university.edu dot in" → "university.edu.in" (real dot + spoken dot)
    let re_multi2 = RE_MULTI2.get_or_init(|| regex::Regex::new(
        r"(?i)(\w+)\.(\w+)\s+dot\s+(\w+)"
    ).unwrap());
    let text = re_multi2.replace_all(&text, |caps: &regex::Captures| {
        format!("{}.{}.{}", &caps[1], &caps[2], &caps[3])
    }).to_string();

    // Step 2: convert email patterns
    // "john at gmail dot com" (full: user@domain.tld).
    // TLD is letters-only so times ("at 7.45") and decimals never become emails.
    let re_dot = RE_DOT.get_or_init(|| regex::Regex::new(
        r"(?i)\b(\w+)\s+at\s+(?:the\s+)?(\w+(?:\s\w+)*?)\s+dot\s+([a-zA-Z]{2,})\b"
    ).unwrap());
    let text = re_dot.replace_all(&text, |caps: &regex::Captures| {
        let user = &caps[1];
        let domain = caps[2].replace(' ', "");
        let tld = &caps[3];
        format!("{}@{}.{}", user, domain, tld)
    }).to_string();

    // "john at the company.com" (period: user@domain.tld)
    let re_dot2 = RE_DOT2.get_or_init(|| regex::Regex::new(
        r"(?i)\b(\w+)\s+at\s+(?:the\s+)?(\w+)\.([a-zA-Z]{2,})\b"
    ).unwrap());
    let text = re_dot2.replace_all(&text, |caps: &regex::Captures| {
        let user = &caps[1];
        let domain = &caps[2];
        let tld = &caps[3];
        format!("{}@{}.{}", user, domain, tld)
    }).to_string();

    // "john at gmail dot" (no TLD — Whisper dropped it) → john@gmail.
    // Domain must contain a letter so "at 7 dot" (decimals/times) is never an email.
    let re_no_tld = RE_NO_TLD.get_or_init(|| regex::Regex::new(
        r"(?i)\b(\w+)\s+at\s+(?:the\s+)?(\w*[a-zA-Z]\w*)\s+dot\b"
    ).unwrap());
    re_no_tld.replace_all(&text, |caps: &regex::Captures| {
        let user = &caps[1];
        let domain = &caps[2];
        format!("{}@{}", user, domain)
    }).to_string()
}

fn cleanup_transcript(text: &str, config: &Config) -> Result<String, String> {
    let url = format!("{}/chat/completions", config.api_base_url.trim_end_matches('/'));

    let client = http_client();

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Message {
        role: String,
        content: Option<String>,
    }

    #[derive(serde::Serialize)]
    struct Request {
        model: String,
        messages: Vec<Message>,
        temperature: f32,
    }

    #[derive(serde::Deserialize)]
    struct Choice {
        message: Message,
    }

    #[derive(serde::Deserialize)]
    struct Response {
        choices: Vec<Choice>,
    }

    let body = Request {
        model: config.cleanup_model.clone(),
        messages: vec![
            Message { role: "system".into(), content: Some(ENHANCE_SYSTEM_PROMPT.into()) },
            Message { role: "user".into(), content: Some(text.into()) },
        ],
        temperature: 0.0,
    };

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(&body)
        .send()
        .map_err(|e| format!("[ENH-001] Cleanup request failed — check your internet. ({e})"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("[ENH-002] Cleanup service error {status}. ({body})"));
    }

    let result: Response = resp.json().map_err(|e| format!("[ENH-003] Could not parse cleanup response. ({e})"))?;

    Ok(result.choices.into_iter().next().and_then(|c| c.message.content).unwrap_or_default())
}

fn remove_fillers(text: &str) -> String {
    static FILLER_RES: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    let regexes = FILLER_RES.get_or_init(|| {
        FILLERS.iter().map(|&filler| {
            regex::Regex::new(&format!(r"\b{}\b", regex::escape(filler))).unwrap()
        }).collect()
    });
    let mut result = text.to_string();
    for re in regexes {
        result = re.replace_all(&result, "").into_owned();
    }
    let result = strip_orphan_commas(&result);
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_orphan_commas(text: &str) -> String {
    // deleting a filler from between punctuation leaves ", ," or ", ." — keep a
    // comma only when it directly follows the previous word ("hello, world")
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ',' && (i == 0 || chars[i - 1].is_whitespace()) {
            continue;
        }
        out.push(c);
    }
    out
}

fn correct_dictionary_terms(text: &str, dictionary: &[String]) -> String {
    if dictionary.is_empty() {
        return text.to_string();
    }
    let terms: Vec<(String, String)> = dictionary.iter()
        .filter(|t| t.chars().count() >= 5)
        .map(|t| (t.clone(), t.to_lowercase()))
        .collect();
    if terms.is_empty() {
        return text.to_string();
    }
    // ponytail: conservative fuzzy token swap — only near-misses within a tight
    // edit distance are replaced ("LightJBM"->"LightGBM"); wildly garbled tokens
    // ("QBundit"->"Optuna") stay as-is. Whisper's prompt bias is the real fix.
    text.split_whitespace()
        .map(|token| {
            let stripped: String = token.chars().filter(|c| c.is_alphanumeric()).collect();
            if stripped.chars().count() < 4 {
                return token.to_string();
            }
            let low = stripped.to_lowercase();
            for (term, tlower) in &terms {
                let tlen = tlower.chars().count();
                let max_dist = if tlen >= 8 { 2 } else { 1 };
                if (low.chars().count() as isize - tlen as isize).abs() > 2 {
                    continue;
                }
                if edit_distance(&low, tlower) <= max_dist {
                    return term.clone();
                }
            }
            token.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn add_punctuation(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    let last = trimmed.chars().last().unwrap();
    if !matches!(last, '.' | '!' | '?' | ',' | ';' | ':') {
        format!("{}.", trimmed)
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            remove_fillers: true,
            auto_punctuation: true,
            cleanup_model: String::new(),
            ..Config::default()
        }
    }

    #[test]
    fn remove_hallucination_loops_collapses_repeats() {
        assert_eq!(remove_hallucination_loops("found output output output yes"), "found output yes");
        assert_eq!(remove_hallucination_loops("luckily found output output output yes"), "luckily found output yes");
        assert_eq!(remove_hallucination_loops("this is normal text"), "this is normal text");
        assert_eq!(remove_hallucination_loops("very very good"), "very very good");
        assert_eq!(remove_hallucination_loops("thank you thank you"), "thank you thank you");
    }

    #[test]
    fn basic_removes_fillers_and_punctuates() {
        let c = test_config();
        let result = postprocess("um hello world", &c);
        assert_eq!(result, "hello world.");
    }

    #[test]
    fn basic_preserves_existing_punctuation() {
        let c = test_config();
        let result = postprocess("hello world!", &c);
        assert_eq!(result, "hello world!");
    }

    #[test]
    fn basic_empty_input_returns_empty() {
        let c = test_config();
        let result = postprocess("", &c);
        assert_eq!(result, "");
    }

    #[test]
    fn no_model_falls_back_to_basic_cleanup() {
        let c = test_config();
        let result = postprocess("hello world", &c);
        assert_eq!(result, "hello world.");
    }

    #[test]
    fn basic_handles_multiple_fillers() {
        let c = test_config();
        let result = postprocess("um like i mean hello world", &c);
        assert_eq!(result, "hello world.");
    }

    #[test]
    fn filler_removal_does_not_leave_double_comma() {
        let c = test_config();
        assert_eq!(remove_fillers("tomorrow, you know, around"), "tomorrow, around");
        assert_eq!(
            postprocess("we should meet tomorrow, you know, around noon", &c),
            "we should meet tomorrow, around noon."
        );
    }

    #[test]
    fn self_correction_strips_marker_and_number() {
        let c = enhanced_config();
        assert_eq!(
            resolve_self_corrections("let's meet at 3 pm no wait i meant 4 pm at the coffee shop"),
            ResolveOutcome::Resolved("let's meet at 4 pm at the coffee shop".into())
        );
        assert_eq!(
            postprocess("let's meet at 3 pm no wait i meant 4 pm at the coffee shop", &c),
            "let's meet at 4 pm at the coffee shop."
        );
    }

    #[test]
    fn self_correction_with_only_i_meant_marker() {
        let c = enhanced_config();
        assert_eq!(
            resolve_self_corrections("let's meet at 3 pm i meant 4 pm at the coffee shop"),
            ResolveOutcome::Resolved("let's meet at 4 pm at the coffee shop".into())
        );
        assert_eq!(
            postprocess("let's meet at 3 pm i meant 4 pm at the coffee shop", &c),
            "let's meet at 4 pm at the coffee shop."
        );
        // a second "i meant" past a value is a genuine chain, not resolvable
        assert_eq!(
            resolve_self_corrections("meet at 3 no wait 4 i meant 5"),
            ResolveOutcome::Ambiguous
        );
    }

    #[test]
    fn dictionary_corrects_near_misses_only() {
        let dict = vec!["LightGBM".into(), "Optuna".into(), "Agmarknet".into()];
        assert_eq!(
            correct_dictionary_terms("I used LightJBM for training", &dict),
            "I used LightGBM for training"
        );
        // garbled beyond a tight edit distance -> left alone, not corrupted
        assert_eq!(
            correct_dictionary_terms("tuned with QBundit and Octono", &dict),
            "tuned with QBundit and Octono"
        );
        // common words are protected
        assert_eq!(
            correct_dictionary_terms("the park near the store", &dict),
            "the park near the store"
        );
    }

    #[test]
    fn dictionary_applies_through_postprocess() {
        let mut c = test_config();
        c.dictionary = vec!["LightGBM".into()];
        let result = postprocess("I used LightJBM for training", &c);
        assert_eq!(result, "I used LightGBM for training.");
    }

    fn enhanced_config() -> Config {
        let mut c = test_config();
        c.cleanup_model = "llama-3.1-8b-instant".into();
        c
    }

    #[test]
    fn self_correction_resolves_locally_without_llama() {
        let c = enhanced_config();
        assert_eq!(
            resolve_self_corrections("meet at 6pm no wait 7pm"),
            ResolveOutcome::Resolved("meet at 7pm".into())
        );
        assert_eq!(postprocess("meet at 6pm no wait 7pm", &c), "meet at 7pm.");
    }

    #[test]
    fn no_marker_skips_llama_and_uses_basic_cleanup() {
        let c = enhanced_config();
        assert_eq!(resolve_self_corrections("hello world"), ResolveOutcome::NoCorrection);
        assert_eq!(postprocess("hello world", &c), basic_cleanup("hello world", &c));
    }

    #[test]
    fn ambiguous_correction_bails_to_llama_path() {
        assert_eq!(resolve_self_corrections("wait"), ResolveOutcome::Ambiguous);
        assert_eq!(
            resolve_self_corrections("meet at 6pm no wait 7pm actually 8pm"),
            ResolveOutcome::Ambiguous
        );
    }

    #[test]
    fn resolves_common_correction_patterns() {
        assert_eq!(resolve_self_corrections("today no tomorrow"), ResolveOutcome::Resolved("tomorrow".into()));
        assert_eq!(resolve_self_corrections("the park no wait the cafe"), ResolveOutcome::Resolved("the cafe".into()));
        assert_eq!(resolve_self_corrections("the park no, wait the cafe"), ResolveOutcome::Resolved("the cafe".into()));
        assert_eq!(resolve_self_corrections("Friday actually Saturday"), ResolveOutcome::Resolved("Saturday".into()));
        assert_eq!(resolve_self_corrections("meet at 5pm instead of 4pm"), ResolveOutcome::Resolved("meet at 5pm".into()));
        assert_eq!(resolve_self_corrections("the park i mean the cafe"), ResolveOutcome::Resolved("the cafe".into()));
        assert_eq!(
            resolve_self_corrections("i will go to school tomorrow no day after tomorrow"),
            ResolveOutcome::Resolved("i will go to school day after tomorrow".into())
        );
    }

    #[test]
    fn convert_spoken_emails_basic() {
        assert_eq!(convert_spoken_emails("send to john at gmail dot com"), "send to john@gmail.com");
        assert_eq!(convert_spoken_emails("email navin at redmail dot com"), "email navin@redmail.com");
        assert_eq!(convert_spoken_emails("manager at outlook dot com"), "manager@outlook.com");
        assert_eq!(convert_spoken_emails("no email here"), "no email here");
        assert_eq!(convert_spoken_emails("manager at the company.com"), "manager@company.com");
        assert_eq!(convert_spoken_emails("john at gmail.com"), "john@gmail.com");
        assert_eq!(convert_spoken_emails("manager at outlook dot ,"), "manager@outlook ,");
        assert_eq!(convert_spoken_emails("team at company dot ."), "team@company .");
        assert_eq!(convert_spoken_emails("user at university dot edu dot in"), "user@university.edu.in");
        assert_eq!(convert_spoken_emails("user at university.edu dot in"), "user@university.edu.in");
        assert_eq!(convert_spoken_emails("user at company dot co dot uk"), "user@company.co.uk");
        assert_eq!(convert_spoken_emails("the exam at 7.45 in the morning"), "the exam at 7.45 in the morning");
        assert_eq!(convert_spoken_emails("meet at 7 dot 45"), "meet at 7 dot 45");
    }
}
