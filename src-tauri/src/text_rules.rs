// Deterministic pre/post text rules — ported from JezFlow TextRules.swift
// (Phase 1: only what the 14 PORTING-PROMPT conformance cases need).
// Pre:  emoji -> numbers -> commands -> scratch -> restarts -> swaps ->
//       repeats -> article -> fillers -> stutter x2
// Post: layout_message -> apply_counted_lists -> question_marks
use regex::{Captures, Regex};
use std::collections::HashSet;
use std::sync::OnceLock;

fn trim_word(s: &str) -> &str {
    // Swift trims CharacterSet.punctuationCharacters ($ is a symbol there and stays).
    s.trim_matches(|c: char| c.is_ascii_punctuation() && c != '$')
}

// ---------------------------------------------------------------------------
// Emoji (longest-name-first table from TextRules.swift)
// ---------------------------------------------------------------------------

static EMOJI_NAMES: &[(&str, &str)] = &[
    ("laugh cry", "\u{1F602}"), ("laughing cry", "\u{1F602}"), ("laugh crying", "\u{1F602}"),
    ("laughing crying", "\u{1F602}"), ("cry laugh", "\u{1F602}"), ("crying laugh", "\u{1F602}"),
    ("crying laughing", "\u{1F602}"), ("cry laughing", "\u{1F602}"), ("tears of joy", "\u{1F602}"),
    ("laughing", "\u{1F602}"), ("laugh", "\u{1F602}"),
    ("rolling on the floor", "\u{1F923}"), ("rofl", "\u{1F923}"), ("happy cry", "\u{1F972}"),
    ("smiling cry", "\u{1F972}"),
    ("sobbing", "\u{1F62D}"), ("loud crying", "\u{1F62D}"), ("bawling", "\u{1F62D}"),
    ("crying", "\u{1F622}"), ("cry", "\u{1F622}"), ("sad", "\u{1F622}"),
    ("heart eyes", "\u{1F60D}"), ("blowing kiss", "\u{1F618}"), ("kiss", "\u{1F618}"),
    ("sweat smile", "\u{1F605}"), ("nervous laugh", "\u{1F605}"),
    ("grinning", "\u{1F601}"), ("grin", "\u{1F601}"), ("smiley", "\u{1F60A}"),
    ("smiling", "\u{1F60A}"), ("smile", "\u{1F60A}"), ("slight smile", "\u{1F642}"),
    ("winking", "\u{1F609}"), ("wink", "\u{1F609}"), ("sunglasses", "\u{1F60E}"),
    ("cool", "\u{1F60E}"), ("thinking", "\u{1F914}"), ("grimace", "\u{1F62C}"),
    ("grimacing", "\u{1F62C}"),
    ("eye roll", "\u{1F644}"), ("rolling eyes", "\u{1F644}"), ("shrug", "\u{1F937}"),
    ("shrugging", "\u{1F937}"), ("skull", "\u{1F480}"), ("neutral", "\u{1F610}"),
    ("sleeping", "\u{1F634}"), ("sleepy", "\u{1F634}"), ("vomit", "\u{1F92E}"),
    ("puke", "\u{1F92E}"), ("partying", "\u{1F973}"), ("party face", "\u{1F973}"),
    ("thumbs up", "\u{1F44D}"), ("thumbs down", "\u{1F44E}"), ("clapping", "\u{1F44F}"),
    ("clap", "\u{1F44F}"), ("praying", "\u{1F64F}"), ("pray", "\u{1F64F}"),
    ("folded hands", "\u{1F64F}"), ("thank you hands", "\u{1F64F}"), ("fire", "\u{1F525}"),
    ("red heart", "\u{2764}\u{FE0F}"), ("heart", "\u{2764}\u{FE0F}"), ("broken heart", "\u{1F494}"),
    ("hundred", "\u{1F4AF}"), ("100", "\u{1F4AF}"), ("party", "\u{1F389}"), ("tada", "\u{1F389}"),
    ("celebration", "\u{1F389}"), ("check", "\u{2705}"), ("tick", "\u{2705}"),
    ("cross", "\u{274C}"), ("rocket", "\u{1F680}"), ("eyes", "\u{1F440}"), ("wave", "\u{1F44B}"),
    ("waving", "\u{1F44B}"), ("muscle", "\u{1F4AA}"), ("flex", "\u{1F4AA}"),
    ("ok hand", "\u{1F44C}"), ("peace", "\u{270C}\u{FE0F}"), ("star", "\u{2B50}"),
    ("sparkles", "\u{2728}"), ("poop", "\u{1F4A9}"), ("money", "\u{1F4B0}"), ("beer", "\u{1F37A}"),
    ("coffee", "\u{2615}"), ("pizza", "\u{1F355}"), ("cake", "\u{1F382}"), ("sun", "\u{2600}\u{FE0F}"),
    ("rain", "\u{1F327}\u{FE0F}"), ("dog", "\u{1F436}"), ("cat", "\u{1F431}"),
];

fn emoji_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let mut names: Vec<&str> = EMOJI_NAMES.iter().map(|&(n, _)| n).collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.chars().count()));
        let alts: Vec<String> = names
            .iter()
            .map(|n| regex::escape(n).replace(' ', r"[\s,\-]+"))
            .collect();
        Regex::new(&format!(
            r"(?i)\s*[,.]?\s*\b(?:(?:a|an|the)\s+)?({})[\s,\-]+emojis?\b[,.]?",
            alts.join("|")
        ))
        .unwrap()
    })
}

fn emoji_key(s: &str) -> String {
    let mut key = String::new();
    let mut prev_sep = false;
    for c in s.chars() {
        if c.is_whitespace() || c == ',' || c == '-' {
            if !prev_sep && !key.is_empty() {
                key.push(' ');
            }
            prev_sep = true;
        } else {
            key.push(c);
            prev_sep = false;
        }
    }
    key.to_lowercase()
}

fn apply_emoji(text: &str) -> String {
    let re = emoji_regex();
    let mut t = text.to_string();
    while let Some(caps) = re.captures(&t) {
        let m = caps.get(0).unwrap();
        let key = emoji_key(caps.get(1).unwrap().as_str());
        let glyph = EMOJI_NAMES
            .iter()
            .find(|(n, _)| *n == key)
            .map(|(_, g)| *g)
            .unwrap_or("");
        let before = &t[..m.start()];
        let sep = if before.is_empty() || before.chars().last().is_some_and(|c| c.is_whitespace()) {
            ""
        } else {
            " "
        };
        t.replace_range(m.start()..m.end(), &format!("{sep}{glyph}"));
    }
    t
}

// ---------------------------------------------------------------------------
// Spoken numbers
// ---------------------------------------------------------------------------

fn small_number(w: &str) -> Option<i64> {
    Some(match w {
        "zero" => 0, "one" => 1, "two" => 2, "three" => 3, "four" => 4, "five" => 5,
        "six" => 6, "seven" => 7, "eight" => 8, "nine" => 9, "ten" => 10,
        "eleven" => 11, "twelve" => 12, "thirteen" => 13, "fourteen" => 14,
        "fifteen" => 15, "sixteen" => 16, "seventeen" => 17, "eighteen" => 18,
        "nineteen" => 19, "twenty" => 20, "thirty" => 30, "forty" => 40,
        "fifty" => 50, "sixty" => 60, "seventy" => 70, "eighty" => 80, "ninety" => 90,
        _ => return None,
    })
}

fn scale_value(w: &str) -> Option<f64> {
    Some(match w {
        "hundred" => 100.0,
        "thousand" => 1_000.0,
        "million" => 1_000_000.0,
        "billion" => 1_000_000_000.0,
        "trillion" => 1_000_000_000_000.0,
        _ => return None,
    })
}

fn big_scale(w: &str) -> Option<&'static str> {
    match w {
        "million" => Some("million"),
        "billion" => Some("billion"),
        "trillion" => Some("trillion"),
        _ => None,
    }
}

fn word_at(tokens: &[String], k: usize) -> String {
    trim_word(&tokens[k].to_lowercase()).to_string()
}

fn trailing_punctuation(s: &str) -> String {
    s.chars()
        .rev()
        .take_while(|&c| c == ',' || c == '.' || c == '?' || c == '!')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn fmt_g(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn format_int(n: i64) -> String {
    if n >= 10_000 {
        let s = n.to_string();
        let b = s.as_bytes();
        let mut out = String::with_capacity(s.len() + s.len() / 3);
        for (i, ch) in b.iter().enumerate() {
            if i > 0 && (b.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(*ch as char);
        }
        out
    } else {
        n.to_string()
    }
}

/// Parses number words starting at `from`: (value, has_decimal, last_big_scale, consumed).
fn parse_number(tokens: &[String], from: usize) -> Option<(f64, bool, Option<&'static str>, usize)> {
    let mut i = from;
    let mut total = 0.0f64;
    let mut current = 0.0f64;
    let mut has_decimal = false;
    let mut last_scale: Option<&'static str> = None;
    let mut saw_word = false;
    let mut decimal_scale = 0.1f64;
    let mut after_point = false;

    // Digit tokens ("20", "1.5", "2,000") count too.
    let w0 = word_at(tokens, i);
    if !w0.is_empty() {
        if let Ok(d) = w0.replace(',', "").parse::<f64>() {
            total = d;
            has_decimal = w0.contains('.');
            i += 1;
            if i < tokens.len() {
                let w = word_at(tokens, i);
                if big_scale(&w).is_some() {
                    total *= scale_value(&w).unwrap();
                    last_scale = big_scale(&w);
                    i += 1;
                }
            }
            return Some((total, has_decimal, last_scale, i - from));
        }
    }

    while i < tokens.len() {
        let w = word_at(tokens, i);
        if after_point {
            if let Some(n) = small_number(&w) {
                if n < 10 {
                    total += n as f64 * decimal_scale;
                    decimal_scale /= 10.0;
                    i += 1;
                    continue;
                }
            }
            if let Some(name) = big_scale(&w) {
                total *= scale_value(&w).unwrap();
                last_scale = Some(name);
                i += 1;
            }
            break;
        }
        if (w == "a" || w == "an")
            && i + 1 < tokens.len()
            && scale_value(&word_at(tokens, i + 1)).is_some()
        {
            current = 1.0;
            i += 1;
            saw_word = true;
            continue;
        }
        if let Some(n) = small_number(&w) {
            current += n as f64;
            i += 1;
            saw_word = true;
            continue;
        }
        if scale_value(&w).is_some() {
            if !saw_word {
                return None;
            }
            if w == "hundred" {
                current = (if current == 0.0 { 1.0 } else { current }) * 100.0;
            } else {
                total += (if current == 0.0 { 1.0 } else { current }) * scale_value(&w).unwrap();
                current = 0.0;
                last_scale = big_scale(&w);
            }
            i += 1;
            continue;
        }
        if w == "and"
            && saw_word
            && i + 1 < tokens.len()
            && (small_number(&word_at(tokens, i + 1)).is_some()
                || scale_value(&word_at(tokens, i + 1)).is_some())
        {
            i += 1;
            continue;
        }
        if w == "point" && saw_word && i + 1 < tokens.len() {
            if let Some(n) = small_number(&word_at(tokens, i + 1)) {
                if n < 10 {
                    after_point = true;
                    has_decimal = true;
                    total += current;
                    current = 0.0;
                    i += 1;
                    continue;
                }
            }
        }
        break;
    }
    if !saw_word {
        return None;
    }
    total += current;
    Some((total, has_decimal, last_scale, i - from))
}

fn apply_numbers(text: &str) -> String {
    let mut tokens: Vec<String> = text.split(' ').map(|s| s.to_string()).collect();
    let mut i = 0;
    while i < tokens.len() {
        let Some((value, has_decimal, last_scale, consumed)) = parse_number(&tokens, i) else {
            i += 1;
            continue;
        };
        let mut end = i + consumed;
        let trailing = trailing_punctuation(&tokens[i + consumed - 1]);
        let mut unit = "";
        let next = if end < tokens.len() {
            word_at(&tokens, end)
        } else {
            String::new()
        };
        let next_trail = if end < tokens.len() {
            trailing_punctuation(&tokens[end])
        } else {
            String::new()
        };
        if next == "dollars" || next == "dollar" || next == "bucks" {
            unit = "$";
            end += 1;
        } else if next == "percent"
            || (next == "per"
                && end + 1 < tokens.len()
                && tokens[end + 1].to_lowercase().starts_with("cent"))
        {
            unit = "%";
            end += if next == "per" { 2 } else { 1 };
        } else if next == "k" {
            unit = "k";
            end += 1;
        }
        let spoken_digits = tokens[i].chars().next().is_some_and(|c| c.is_ascii_digit());
        let worth = value >= 10.0 || has_decimal || !unit.is_empty() || last_scale.is_some();
        // Small bare counts ("two things") stay as words; digits without a
        // unit/scale stay as they were dictated.
        if !worth || (spoken_digits && unit.is_empty() && last_scale.is_none()) {
            i += 1;
            continue;
        }
        let body = match last_scale {
            Some(scale) => {
                let n = value / scale_value(scale).unwrap();
                let ns = if n.fract() == 0.0 {
                    format!("{}", n as i64)
                } else {
                    fmt_g(n)
                };
                format!("{ns} {scale}")
            }
            None => {
                if has_decimal {
                    fmt_g(value)
                } else {
                    format_int(value as i64)
                }
            }
        };
        let mut replacement = if unit == "$" {
            format!("${body}")
        } else {
            format!("{body}{}", if unit == "%" || unit == "k" { unit } else { "" })
        };
        replacement.push_str(if unit.is_empty() { &trailing } else { &next_trail });
        tokens.splice(i..end, std::iter::once(replacement));
        i += 1;
    }
    tokens.join(" ")
}

// ---------------------------------------------------------------------------
// Spoken commands
// ---------------------------------------------------------------------------

fn apply_commands(text: &str) -> String {
    static WORD_CMDS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    static EMAIL: OnceLock<Regex> = OnceLock::new();
    static URL: OnceLock<Regex> = OnceLock::new();
    static DOT: OnceLock<Regex> = OnceLock::new();

    let cmds = WORD_CMDS.get_or_init(|| {
        vec![
            (Regex::new(r"(?i)[,.]?\s*\b(?:new paragraph|paragraph break)\b[,.]?\s*").unwrap(), "\n\n"),
            (Regex::new(r"(?i)[,.]?\s*\b(?:new line|newline|line break)\b[,.]?\s*").unwrap(), "\n"),
            (Regex::new(r"(?i)[,.]?\s*\b(?:bullet point|dot point|new bullet|next bullet)\b[,.]?\s*").unwrap(), "\n- "),
            (Regex::new(r"(?i)\s*\b(?:full stop|fullstop)\b[,.]?").unwrap(), "."),
            (Regex::new(r"(?i)\s*\bquestion mark\b[,.]?").unwrap(), "?"),
            (Regex::new(r"(?i)\s*\bexclamation (?:mark|point)\b[,.]?").unwrap(), "!"),
        ]
    });
    // Email / URL / dot passes stay case-sensitive like the Swift reference.
    let email = EMAIL.get_or_init(|| {
        Regex::new(r"\b([\w.-]+) at ([\w-]+)((?: dot [a-z]{2,4})+)\b").unwrap()
    });
    let url = URL
        .get_or_init(|| Regex::new(r"\b([\w-]+)((?: dot [a-z]{2,4})+)\b").unwrap());
    let dot = DOT.get_or_init(|| Regex::new(r" dot ([a-z]{2,4})\b").unwrap());

    let mut t = text.to_string();
    for (re, rep) in cmds {
        t = re.replace_all(&t, *rep).into_owned();
    }
    // "sam at example dot com" -> sam@example.com ; second pass turns the
    // " dot xx" runs left behind into ".xx".
    t = email
        .replace_all(&t, |c: &Captures| format!("{}@{}{}", &c[1], &c[2], &c[3]))
        .into_owned();
    t = url
        .replace_all(&t, |c: &Captures| format!("{}{}", &c[1], &c[2]))
        .into_owned();
    t = dot
        .replace_all(&t, |c: &Captures| format!(".{}", &c[1]))
        .into_owned();
    t.replace(" \n", "\n").trim().to_string()
}

// ---------------------------------------------------------------------------
// Scratch, restarts, swaps, repeats, article
// ---------------------------------------------------------------------------

fn apply_scratch(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:actually\s+)?(?:scratch that|forget that|never ?mind)\b[,.!?]?\s*")
            .unwrap()
    });
    let is_term = |c: char| c == '.' || c == '!' || c == '?';
    let mut t = text.to_string();
    while let Some(m) = re.find(&t) {
        let chars: Vec<char> = t[..m.start()].chars().collect();
        let mut i = chars.len();
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        let sentence_start = i == 0 || is_term(chars[i - 1]);
        if sentence_start && i > 0 {
            i -= 1;
        }
        while i > 0 && !is_term(chars[i - 1]) {
            i -= 1;
        }
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let keep: String = chars[..i].iter().collect();
        t = keep + &t[m.end()..];
    }
    t
}

static SWAP_CLASSES: &[&[&str]] = &[
    &["he", "she", "they", "him", "her", "them", "his", "hers", "their", "i", "we", "you", "me", "us"],
    &["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"],
    &["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"],
    &["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"],
    &["morning", "afternoon", "evening", "tonight", "today", "tomorrow", "yesterday"],
];

fn apply_swaps(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\b([\w']+)[,.]?\s+(?:actually|no|sorry|i mean)[,.]?\s+([\w']+)\b")
            .unwrap()
    });
    let mut t = text.to_string();
    let mut from = 0;
    while from < t.len() {
        let Some(caps) = t.get(from..).and_then(|s| re.captures(s)) else {
            break;
        };
        let m = caps.get(0).unwrap();
        let abs_start = from + m.start();
        let abs_end = from + m.end();
        let x = caps[1].to_lowercase();
        let y = caps[2].to_string();
        let yl = y.to_lowercase();
        let same_class = x != yl
            && (SWAP_CLASSES
                .iter()
                .any(|c| c.contains(&x.as_str()) && c.contains(&yl.as_str()))
                || (x.parse::<i32>().is_ok() && yl.parse::<i32>().is_ok()));
        if same_class {
            t.replace_range(abs_start..abs_end, &y);
            from = abs_start + y.len();
        } else {
            from = abs_start + t[abs_start..].chars().next().map_or(1, |c| c.len_utf8());
        }
    }
    t
}

fn apply_restarts(text: &str) -> String {
    static SIG: OnceLock<Regex> = OnceLock::new();
    static WORD: OnceLock<Regex> = OnceLock::new();
    let sig_re = SIG.get_or_init(|| {
        Regex::new(
            r"(?i)[,.]?\s*\b(?:sorry|no[,\s]+wait|wait[,\s]+no|i mean|correction|let me rephrase)\b[,.]?\s*",
        )
        .unwrap()
    });
    let word_re = WORD.get_or_init(|| Regex::new(r"[^\s]+").unwrap());
    static STARTERS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let starters = STARTERS.get_or_init(|| {
        [
            "because", "so", "and", "but", "if", "when", "then", "can", "could", "would",
            "should", "i", "we", "you", "he", "she", "they", "it", "the", "to", "let's",
            "please", "send", "tell",
        ]
        .into_iter()
        .collect()
    });
    let norm = |w: &str| trim_word(&w.to_lowercase()).to_string();

    let mut t = text.to_string();
    let mut from = 0;
    while from < t.len() {
        let Some(m0) = t.get(from..).and_then(|s| sig_re.find(s)) else {
            break;
        };
        let sig_start = from + m0.start();
        let sig_end = from + m0.end();
        let head = &t[..sig_start];
        let mut before: Vec<(usize, usize)> = word_re
            .find_iter(head)
            .map(|m| (m.start(), m.end()))
            .collect();
        if let Some(end_idx) = before.iter().rposition(|&(_, e)| {
            head[..e].ends_with(['.', '!', '?'])
        }) {
            before = before[end_idx + 1..].to_vec();
        }
        if before.len() > 12 {
            before = before[before.len() - 12..].to_vec();
        }
        let after: Vec<String> = word_re
            .find_iter(&t[sig_end..])
            .take(6)
            .map(|m| t[sig_end + m.start()..sig_end + m.end()].to_string())
            .collect();
        let bw = |k: usize| norm(&head[before[k].0..before[k].1]);
        let mut best_k = -1isize;
        let mut best_len = 0usize;
        for k in 0..before.len() {
            let mut len = 0;
            while k + len < before.len() && len < after.len() && bw(k + len) == norm(&after[len]) {
                len += 1;
            }
            if len > best_len {
                best_len = len;
                best_k = k as isize;
            }
        }
        let accept = best_len >= 2
            || (best_len == 1 && best_k >= 0 && starters.contains(bw(best_k as usize).as_str()));
        if accept {
            let start = before[best_k as usize].0;
            t.replace_range(start..sig_end, "");
            from = start;
        } else {
            from = sig_end;
        }
    }
    t
}

fn apply_repeats(text: &str) -> String {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let word_re = WORD.get_or_init(|| Regex::new(r"[^\s]+").unwrap());
    static CONJ: OnceLock<HashSet<&'static str>> = OnceLock::new();
    let conj = CONJ.get_or_init(|| {
        ["and", "or", "but", "then", "so", "because", "if", "when"]
            .into_iter()
            .collect()
    });
    let words: Vec<(usize, usize)> = word_re.find_iter(text).map(|m| (m.start(), m.end())).collect();
    if words.len() < 6 {
        return text.to_string();
    }
    let norms: Vec<String> = words
        .iter()
        .map(|&(s, e)| trim_word(&text[s..e].to_lowercase()).to_string())
        .collect();
    for i in 0..words.len() - 5 {
        for gap in 0..=4 {
            let j = i + 3 + gap;
            if j + 2 >= words.len() {
                continue;
            }
            if norms[i] == norms[j]
                && norms[i + 1] == norms[j + 1]
                && norms[i + 2] == norms[j + 2]
                && !conj.contains(norms[j - 1].as_str())
                && !norms[i].is_empty()
            {
                let mut t = text.to_string();
                t.replace_range(words[i].0..words[j].0, "");
                return apply_repeats(&t);
            }
        }
    }
    text.to_string()
}

/// "fair a fair bit" -> "a fair bit": a word restarted with its article.
/// (Rust's regex crate has no backrefs/lookbehind — done by hand.)
fn apply_article_restart(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\b(\w+)[,]?\s+(a|an|the)\s+(\w+)\b").unwrap()
    });
    const BLOCKED: &[&str] = &["the", "a", "an", "of", "in", "on", "at", "by", "to", "for"];
    // Swift's (?<!\b(?:the|a|...)\s): exactly one whitespace then the blocked
    // word (with a word boundary) must NOT sit immediately before the match.
    let blocked_before = |prefix: &str| -> bool {
        let chars: Vec<char> = prefix.chars().collect();
        if chars.len() < 2
            || !chars[chars.len() - 1].is_whitespace()
            || chars[chars.len() - 2].is_whitespace()
        {
            return false;
        }
        let mut j = chars.len() - 2;
        while j > 0 && !chars[j - 1].is_whitespace() {
            j -= 1;
        }
        if j > 0 && (chars[j - 1].is_alphanumeric() || chars[j - 1] == '_') {
            return false;
        }
        let word: String = chars[j..chars.len() - 1].iter().collect();
        BLOCKED.iter().any(|b| word.eq_ignore_ascii_case(b))
    };

    let mut t = text.to_string();
    let mut pos = 0;
    while pos < t.len() {
        let Some(caps) = t.get(pos..).and_then(|s| re.captures(s)) else {
            break;
        };
        let m = caps.get(0).unwrap();
        let abs_s = pos + m.start();
        let abs_e = pos + m.end();
        let (w1, art, w3) = (&caps[1], &caps[2], &caps[3]);
        if w1.eq_ignore_ascii_case(w3) && !blocked_before(&t[..abs_s]) {
            let repl = format!("{art} {w1}");
            t.replace_range(abs_s..abs_e, &repl);
            pos = abs_s + repl.len();
        } else {
            pos = abs_s + t[abs_s..].chars().next().map_or(1, |c| c.len_utf8());
        }
    }
    t
}

// ---------------------------------------------------------------------------
// Fillers + stutter (end of the pre pipeline)
// ---------------------------------------------------------------------------

/// One stutter collapse: "we we ship" -> "we ship". Hand-rolled because the
/// regex crate has no backreferences; the Swift lookahead (?=[\s,.!?]|$) is
/// checked manually after the match.
fn apply_stutter(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)\b(\w+)([,.]?\s+)(\w+)\b").unwrap());
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for caps in re.captures_iter(text) {
        let m = caps.get(0).unwrap();
        let w1 = caps.get(1).unwrap().as_str();
        let w3 = caps.get(3).unwrap().as_str();
        let after_ok = text[m.end()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || matches!(c, '.' | ',' | '!' | '?'))
            || m.end() == text.len();
        if w1.eq_ignore_ascii_case(w3) && after_ok {
            out.push_str(&text[last..m.start()]);
            out.push_str(w1);
            last = m.end();
        }
    }
    out.push_str(&text[last..]);
    out
}

fn strip_fillers(text: &str) -> String {
    static FILLER: OnceLock<Regex> = OnceLock::new();
    let filler = FILLER.get_or_init(|| {
        Regex::new(r"(?i)(^|[\s,])(?:um+|uh+|uhm|erm?|hmm+|mm+)\b[,.]?\s*").unwrap()
    });

    let mut t = apply_scratch(text);
    t = apply_restarts(&t);
    t = apply_swaps(&t);
    t = apply_repeats(&t);
    t = apply_article_restart(&t);
    t = filler.replace_all(&t, "$1").into_owned();
    for _ in 0..2 {
        t = apply_stutter(&t);
    }
    // ponytail: collapse only horizontal whitespace — JezFlow's \s{2,} would
    // eat the "\n\n" that "new paragraph" just produced.
    static HSPACE: OnceLock<Regex> = OnceLock::new();
    let hs = HSPACE.get_or_init(|| Regex::new(r"[ \t]{2,}").unwrap());
    t = hs.replace_all(&t, " ").into_owned();
    static LEAD: OnceLock<Regex> = OnceLock::new();
    let lead = LEAD.get_or_init(|| Regex::new(r"^[,\s]+").unwrap());
    t = lead.replace(&t, "").into_owned();
    t.trim().to_string()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Deterministic pre-model pass (spoken commands, fillers, self-corrections).
pub fn apply_pre(text: &str) -> String {
    let t = apply_commands(&apply_numbers(&apply_emoji(text)));
    strip_fillers(&t)
}

/// Deterministic post-model pass (layout only — no reordering of words).
pub fn apply_post(text: &str) -> String {
    let t = layout_message(text);
    let t = apply_counted_lists(&t);
    question_marks(&t)
}

// ---------------------------------------------------------------------------
// Layout (post)
// ---------------------------------------------------------------------------

fn layout_message(text: &str) -> String {
    if text.contains('\n') {
        return text.to_string();
    }
    static G: OnceLock<Regex> = OnceLock::new();
    static S: OnceLock<Regex> = OnceLock::new();
    let g = G.get_or_init(|| {
        // Swift's (?=\s) after a bare name dropped: a greeting running into
        // EOS never pairs with a signoff anyway, so the outcome is the same.
        Regex::new(
            r"(?i)^((?:dear|hi|hey|hello|good morning|good afternoon|good evening|morning|afternoon|evening)\b(?:[^,.!\n]{0,40}[,.!]|\s+[A-Za-z][\w'-]*))\s*",
        )
        .unwrap()
    });
    let s = S.get_or_init(|| {
        Regex::new(
            r"(?i)\s+((?:kind regards|best regards|warm regards|warmest regards|regards|cheers|many thanks|thanks|thank you|best|all the best|talk soon|take care|speak soon)[,.]?)(?:\s+([A-Za-z][\w'-]*(?:\s+[A-Za-z][\w'-]*)?))?[.]?\s*$",
        )
        .unwrap()
    });
    let Some(gc) = g.captures(text) else {
        return text.to_string();
    };
    let gm = gc.get(0).unwrap();
    let greeting = gc.get(1).unwrap().as_str().to_string();
    let after_greeting = gm.end();
    let Some(sc) = s.captures(text) else {
        return text.to_string();
    };
    let sm = sc.get(0).unwrap();
    if sm.start() <= after_greeting {
        return text.to_string();
    }
    let signoff = sc.get(1).unwrap().as_str().to_string();
    let mut body = text[after_greeting..sm.start()].trim().to_string();
    if body.ends_with(',') {
        body.pop();
    }
    if body.split(' ').filter(|w| !w.is_empty()).count() < 2 {
        return text.to_string();
    }
    let mut out = format!("{greeting}\n\n{body}\n\n{signoff}");
    if let Some(name) = sc.get(2) {
        out.push('\n');
        out.push_str(name.as_str());
    }
    out
}

/// First sentence end in `rest`: (byte index of terminator, byte index after
/// it). Mirrors Swift `[.!?](?=\s|$)|\n` without lookaheads.
fn find_sentence_end(rest: &str) -> Option<(usize, usize)> {
    for (i, c) in rest.char_indices() {
        let end = i + c.len_utf8();
        if c == '\n' {
            return Some((i, end));
        }
        if c == '.' || c == '?' || c == '!' {
            match rest[end..].chars().next() {
                None => return Some((i, end)),
                Some(n) if n.is_whitespace() => return Some((i, end)),
                _ => {}
            }
        }
    }
    None
}

fn apply_counted_lists(text: &str) -> String {
    if text.contains("\n1.") || text.contains("\n- ") {
        return text.to_string();
    }
    static LEAD: OnceLock<Regex> = OnceLock::new();
    static JOIN: OnceLock<Regex> = OnceLock::new();
    static AND: OnceLock<Regex> = OnceLock::new();
    static LAST_AND: OnceLock<Regex> = OnceLock::new();
    let lead_re = LEAD.get_or_init(|| {
        Regex::new(
            r"(?i)\b(two|three|four|five|six|2|3|4|5|6)\s+(things|reasons|points|steps|options|ways|issues|updates|questions|items|ideas|problems|tips|takeaways|decisions|requests|asks)\b([^:.\n]{0,80})[:.]\s*",
        )
        .unwrap()
    });
    let join_re = JOIN.get_or_init(|| Regex::new(r"\s*;\s*").unwrap());
    let and_re = AND.get_or_init(|| Regex::new(r",\s*(?:and|or)\s+").unwrap());
    let last_and = LAST_AND.get_or_init(|| Regex::new(r"\s+(?:and|or)\s+").unwrap());

    let n = match caps_count(lead_re, text) {
        Some(n) => n,
        None => return text.to_string(),
    };
    let caps = lead_re.captures(text).unwrap();
    let whole = caps.get(0).unwrap();
    let rest = &text[whole.end()..];
    let (body, tail) = match find_sentence_end(rest) {
        Some((end, after)) => (&rest[..end], rest[after..].trim()),
        None => (rest, ""),
    };
    let body = join_re.replace_all(body, ",");
    let body = and_re.replace_all(&body, ",");
    let mut items: Vec<String> = body
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if items.len() == n - 1 {
        if let Some(last) = items.last().cloned() {
            if let Some(m) = last_and.find(&last) {
                items.pop();
                items.push(last[..m.start()].to_string());
                items.push(last[m.end()..].to_string());
            }
        }
    }
    if items.len() != n {
        return text.to_string();
    }
    let mut lead = text[..whole.end()].trim().to_string();
    if lead.ends_with('.') {
        lead.pop();
        lead.push(':');
    }
    let list: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, it)| format!("{}. {}", i + 1, it))
        .collect();
    let tail = if tail.is_empty() {
        String::new()
    } else {
        format!("\n{tail}")
    };
    format!("{lead}\n{}{tail}", list.join("\n"))
}

fn caps_count(lead_re: &Regex, text: &str) -> Option<usize> {
    let caps = lead_re.captures(text)?;
    match caps[1].to_lowercase().as_str() {
        "two" | "2" => Some(2),
        "three" | "3" => Some(3),
        "four" | "4" => Some(4),
        "five" | "5" => Some(5),
        "six" | "6" => Some(6),
        _ => None,
    }
}

fn question_marks(text: &str) -> String {
    static QS: OnceLock<Regex> = OnceLock::new();
    static LIST: OnceLock<Regex> = OnceLock::new();
    let qs = QS.get_or_init(|| {
        Regex::new(
            r"(?i)^(?:(?:are|can|could|do|does|did|is|will|would|should|shall|have|has|were|was|am)\s+(?:you|we|i|it|there|they|he|she|this|that|anyone|someone)\b|(?:what|when|where|who|why|how)['’](?:s|re|d|ll)\s+\w+|(?:what|when|where|who|why|how)\s+(?:is|are|do|does|did|can|could|will|would|should|have|has|was|were|about|time|many|much|long)\b)",
        )
        .unwrap()
    });
    let list_re = LIST.get_or_init(|| Regex::new(r"^(?:- |\d+[.)] )").unwrap());

    text.split('\n')
        .map(|line| {
            let mut out = String::with_capacity(line.len());
            let mut sentence = String::new();
            let flush = |out: &mut String, sentence: &mut String, terminator: Option<char>| {
                let body = sentence.trim();
                let body = list_re.replace(body, "");
                let is_q = qs.is_match(&body);
                out.push_str(sentence);
                if is_q && terminator.is_none_or(|t| t == '.') {
                    out.push('?');
                } else if let Some(t) = terminator {
                    out.push(t);
                }
                sentence.clear();
            };
            for ch in line.chars() {
                if ch == '.' || ch == '?' || ch == '!' {
                    flush(&mut out, &mut sentence, Some(ch));
                } else {
                    sentence.push(ch);
                }
            }
            if !sentence.trim().is_empty() {
                flush(&mut out, &mut sentence, None);
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Faithfulness guardrail
// ---------------------------------------------------------------------------

fn words_of(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

/// True when `output` looks like a rewrite/hallucination rather than a
/// cleanup of `input` (TextRules.looksUnfaithful).
pub fn looks_unfaithful(input: &str, output: &str, allowed_terms: &[String]) -> bool {
    let in_words = words_of(input);
    let out_words = words_of(output);
    if in_words.len() < 6 {
        return false; // too short to judge
    }
    let ratio = out_words.len() as f64 / in_words.len() as f64;
    if ratio < 0.35 || ratio > 1.6 {
        return true;
    }
    let allowed: HashSet<String> = allowed_terms.iter().flat_map(|t| words_of(t)).collect();
    let in_set: HashSet<&str> = in_words.iter().map(|s| s.as_str()).collect();
    let novel = out_words
        .iter()
        .filter(|w| {
            w.chars().count() > 3 && !in_set.contains(w.as_str()) && !allowed.contains(w.as_str())
        })
        .count();
    novel as f64 / out_words.len().max(1) as f64 > 0.3
}

// ---------------------------------------------------------------------------
// Conformance tests — PORTING-PROMPT.md §9 (14 cases)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case01_leading_filler() {
        assert_eq!(
            apply_pre("Um, so, tomorrow is fine. See you then."),
            "so, tomorrow is fine. See you then."
        );
    }

    #[test]
    fn case02_stutter_collapse() {
        assert_eq!(
            apply_pre("we we we ship on tuesday tomorrow, tomorrow we look"),
            "we ship on tuesday tomorrow we look"
        );
    }

    #[test]
    fn case03_newline_paragraph() {
        assert_eq!(
            apply_pre("first line new line second line, new paragraph third"),
            "first line\nsecond line\n\nthird"
        );
    }

    #[test]
    fn case04_spoken_email() {
        assert_eq!(
            apply_pre("send it to sam at example dot com today"),
            "send it to sam@example.com today"
        );
    }

    #[test]
    fn case05_scratch_that() {
        assert_eq!(
            apply_pre("Hey so tomorrow, tomorrow is fine, actually scratch that, let's do Thursday instead."),
            "let's do Thursday instead."
        );
    }

    #[test]
    fn case06_restart_sentence() {
        assert_eq!(
            apply_pre("today's weather is very quiet sorry today's weather is very sunny"),
            "today's weather is very sunny"
        );
    }

    #[test]
    fn case07_article_restart() {
        assert_eq!(
            apply_pre("This is fair a fair bit of work"),
            "This is a fair bit of work"
        );
    }

    #[test]
    fn case08_swap_pronoun() {
        assert_eq!(
            apply_pre("i caught a cold from my partner and yeah she actually he was also sick"),
            "i caught a cold from my partner and yeah he was also sick"
        );
    }

    #[test]
    fn case09_repeat_false_start() {
        assert_eq!(
            apply_pre("look mate i have a thesing what thief i have a few things i want to discuss"),
            "look mate i have a few things i want to discuss"
        );
    }

    #[test]
    fn case10_spoken_numbers() {
        assert_eq!(
            apply_pre("whisperflow is valued at two billion dollars and granola around one point five billion."),
            "whisperflow is valued at $2 billion and granola around 1.5 billion."
        );
    }

    #[test]
    fn case11_spoken_emoji() {
        assert_eq!(
            apply_pre("that was so funny laugh cry emoji, anyway see you"),
            "that was so funny \u{1F602} anyway see you"
        );
    }

    #[test]
    fn case12_counted_list() {
        assert_eq!(
            apply_post("hey Sam, there are three things it does really well: it's fast, it's accurate, it gets names right."),
            "hey Sam, there are three things it does really well:\n1. it's fast\n2. it's accurate\n3. it gets names right"
        );
    }

    #[test]
    fn case13_message_layout() {
        // SelfTest.swift keeps the comma on the sign-off ("kind regards,\njess");
        // PORTING-PROMPT's hand-written expectation drops it — reference impl wins.
        assert_eq!(
            apply_post("dear robin, no worries at all, i can deal with it this weekend. kind regards, jess"),
            "dear robin,\n\nno worries at all, i can deal with it this weekend.\n\nkind regards,\njess"
        );
    }

    #[test]
    fn case14_question_mark() {
        assert_eq!(
            apply_post("hey alex, quick question. are you guys able to provide a free account"),
            "hey alex, quick question. are you guys able to provide a free account?"
        );
    }

    #[test]
    fn guard_looks_unfaithful() {
        let raw = "okay so the plan is we ship on thursday and then we do the reel for instagram";
        assert!(!looks_unfaithful(
            raw,
            "Okay, so the plan is we ship on Thursday and then we do the reel for Instagram.",
            &[]
        ));
        assert!(looks_unfaithful(
            raw,
            "Sure! Here is a cleaned up version of your message about shipping.",
            &[]
        ));
        assert!(looks_unfaithful(raw, "Ship Thursday.", &[]));
        let dict = vec!["Acme Hub".to_string()];
        assert!(!looks_unfaithful(
            "send the acme hub report to alex by friday please thanks",
            "send the Acme Hub report to alex by friday please thanks",
            &dict
        ));
    }
}
