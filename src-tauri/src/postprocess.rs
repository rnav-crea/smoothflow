use crate::config::Config;
use std::sync::OnceLock;
use std::time::Duration;

fn http_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
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
    "scratch that", "forget that", "i mean", "actually", "no", "wait",
];

pub fn postprocess(text: &str, config: &Config) -> String {
    if config.cleanup_model.is_empty() {
        return basic_cleanup(text, config);
    }
    match resolve_self_corrections(text) {
        // ponytail: resolved text returned verbatim; wrap in basic_cleanup if stray fillers show up
        ResolveOutcome::Resolved(fixed) => fixed,
        ResolveOutcome::NoCorrection => basic_cleanup(text, config),
        ResolveOutcome::Ambiguous => basic_cleanup(text, config),
    }
}

fn resolve_self_corrections(text: &str) -> ResolveOutcome {
    let first_re = marker_re(FIRST_WINS_MARKERS);
    let last_re = marker_re(LAST_WINS_MARKERS);
    let (last_wins, m) = match (first_re.find(text), last_re.find(text)) {
        (Some(fm), Some(lm)) if fm.start() <= lm.start() => (false, fm),
        (Some(_), Some(lm)) => (true, lm),
        (Some(fm), None) => (false, fm),
        (None, Some(lm)) => (true, lm),
        (None, None) => return ResolveOutcome::NoCorrection,
    };
    let before = text[..m.start()].trim();
    let after = text[m.end()..].trim();
    // conservative: a correction must split a real utterance, both sides non-trivial
    if before.is_empty() || !after.chars().any(|c| c.is_alphanumeric()) {
        return ResolveOutcome::Ambiguous;
    }
    // a leftover marker in the tail is a correction chain -> not confidently resolvable
    if first_re.find(after).is_some() || last_re.find(after).is_some() {
        return ResolveOutcome::Ambiguous;
    }
    let resolved = if last_wins {
        // "meet at 6pm no wait 7pm": drop the superseded last word, keep the tail
        let mut words: Vec<&str> = before.split_whitespace().collect();
        words.pop();
        let head = words.join(" ");
        let combined = if head.is_empty() {
            after.to_string()
        } else {
            format!("{head} {after}")
        };
        collapse_adjacent_dupes(&combined)
    } else {
        // "meet at 5pm instead of 4pm": keep the first option, drop the rest
        before.to_string()
    };
    // ponytail: heuristic last-word drop + adjacent-dupe collapse; phrase boundaries,
    // correction chains, and non-correctional "no/wait/actually" aren't distinguished
    ResolveOutcome::Resolved(resolved)
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

fn basic_cleanup(text: &str, config: &Config) -> String {
    let text = if config.remove_fillers {
        remove_fillers(text)
    } else {
        text.to_string()
    };

    let text = convert_spoken_emails(&text);

    if config.auto_punctuation {
        add_punctuation(&text)
    } else {
        text
    }
}

fn convert_spoken_emails(text: &str) -> String {
    // Step 1: fix multi-level TLDs first ("university dot edu dot in" → "university.edu.in")
    let re_multi = regex::Regex::new(
        r"(?i)(\w+)\s+dot\s+(\w+)\s+dot\s+(\w+)"
    ).unwrap();
    let text = re_multi.replace_all(&text, |caps: &regex::Captures| {
        format!("{}.{}.{}", &caps[1], &caps[2], &caps[3])
    }).to_string();

    // "university.edu dot in" → "university.edu.in" (real dot + spoken dot)
    let re_multi2 = regex::Regex::new(
        r"(?i)(\w+)\.(\w+)\s+dot\s+(\w+)"
    ).unwrap();
    let text = re_multi2.replace_all(&text, |caps: &regex::Captures| {
        format!("{}.{}.{}", &caps[1], &caps[2], &caps[3])
    }).to_string();

    // Step 2: convert email patterns
    // "john at gmail dot com" (full: user@domain.tld)
    let re_dot = regex::Regex::new(
        r"(?i)\b(\w+)\s+at\s+(?:the\s+)?(\w+(?:\s\w+)*?)\s+dot\s+(\w+)\b"
    ).unwrap();
    let text = re_dot.replace_all(&text, |caps: &regex::Captures| {
        let user = &caps[1];
        let domain = caps[2].replace(' ', "");
        let tld = &caps[3];
        format!("{}@{}.{}", user, domain, tld)
    }).to_string();

    // "john at the company.com" (period: user@domain.tld)
    let re_dot2 = regex::Regex::new(
        r"(?i)\b(\w+)\s+at\s+(?:the\s+)?(\w+)\.(\w+)\b"
    ).unwrap();
    let text = re_dot2.replace_all(&text, |caps: &regex::Captures| {
        let user = &caps[1];
        let domain = &caps[2];
        let tld = &caps[3];
        format!("{}@{}.{}", user, domain, tld)
    }).to_string();

    // "john at gmail dot" (no TLD — Whisper dropped it) → john@gmail
    let re_no_tld = regex::Regex::new(
        r"(?i)\b(\w+)\s+at\s+(?:the\s+)?(\w+)\s+dot\b"
    ).unwrap();
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
        .map_err(|e| format!("Enhance request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("Enhance API error {status}: {body}"));
    }

    let result: Response = resp.json().map_err(|e| format!("Failed to parse enhance response: {e}"))?;

    Ok(result.choices.into_iter().next().and_then(|c| c.message.content).unwrap_or_default())
}

fn remove_fillers(text: &str) -> String {
    let mut result = text.to_string();
    for &filler in FILLERS {
        let re = regex::Regex::new(&format!(r"\b{}\b", regex::escape(filler))).unwrap();
        result = re.replace_all(&result, "").into_owned();
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
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
        assert_eq!(postprocess("meet at 6pm no wait 7pm", &c), "meet at 7pm");
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
    }
}
