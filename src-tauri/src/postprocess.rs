use crate::config::Config;

const FILLERS: &[&str] = &[
    "um", "uh", "like", "you know", "actually", "basically",
    "literally", "sort of", "kind of", "i mean", "well",
];

const CLEANUP_SYSTEM_PROMPT: &str = "You clean up raw dictation transcripts into polished natural text.\n\n\
Rules:\n\
1. RESOLVE SELF-CORRECTIONS: keep only the final intent.\n\
   \"I will meet at 6pm no wait 7pm\" -> \"I will meet at 7pm\"\n\
   \"today no tomorrow\" -> \"tomorrow\"\n\
   \"Friday actually Saturday\" -> \"Saturday\"\n\
   \"12345 but no its 54321\" -> \"54321\"\n\
\n\
2. FIX CLUNKY PHRASING: rewrite awkward literal speech into natural English.\n\
   \"I will reach the office\" -> \"I will get to the office\" or \"I will arrive\"\n\
   \"because traffic is heavy\" as a reason for a time -> separate into two sentences\n\
\n\
3. EMAIL ADDRESSES: convert spoken \"at\" to @ when clearly an email.\n\
   \"navin at redmail.com\" -> \"navin@redmail.com\"\n\
\n\
4. PUNCTUATION: use natural speech punctuation, not formal writing.\n\
   Avoid semicolons — use periods or \"and\" instead.\n\
   \"send the report; join the meeting\" -> \"send the report and join the meeting\"\n\
\n\
5. TIMES: \"6pm\" -> \"6:00 PM\"\n\
\n\
6. CHOICES: \"X or Y\" -> keep Y (last option).\n\
   \"cake or maybe pizza not sure\" -> \"pizza\"\n\
\n\
7. CORRECTIONS: \"X no it is Y\" -> keep Y.\n\
   \"park no it is cafe\" -> \"cafe\"\n\
\n\
8. CONTRADICTIONS: \"X but also Y\" -> pick the more positive/definitive.\n\
   \"happy but also sad\" -> \"happy\"\n\
\n\
9. Keep first-person perspective. Output ONLY the transcript, no explanations.";

pub fn postprocess(text: &str, config: &Config) -> String {
    if !config.cleanup_model.is_empty() {
        return cleanup_transcript(text, config)
            .unwrap_or_else(|e| {
                eprintln!("CLEANUP ERROR: {e}");
                text.to_string()
            });
    }

    let text = if config.remove_fillers {
        remove_fillers(text)
    } else {
        text.to_string()
    };

    if config.auto_punctuation {
        add_punctuation(&text)
    } else {
        text
    }
}

fn cleanup_transcript(text: &str, config: &Config) -> Result<String, String> {
    let url = format!("{}/chat/completions", config.api_base_url.trim_end_matches('/'));

    let client = reqwest::blocking::Client::new();

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
            Message { role: "system".into(), content: Some(CLEANUP_SYSTEM_PROMPT.into()) },
            Message { role: "user".into(), content: Some(text.into()) },
        ],
        temperature: 0.0,
    };

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(&body)
        .send()
        .map_err(|e| format!("Cleanup request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("Cleanup API error {status}: {body}"));
    }

    let result: Response = resp.json().map_err(|e| format!("Failed to parse cleanup response: {e}"))?;

    Ok(result.choices.into_iter().next().and_then(|c| c.message.content).unwrap_or_default())
}

fn remove_fillers(text: &str) -> String {
    let mut result = text.to_string();
    for filler in FILLERS {
        let pattern_lower = format!(" {} ", filler);
        let pattern_upper = {
            let mut c = filler.chars();
            let first = c.next().unwrap().to_uppercase().to_string();
            format!(" {} ", first + c.as_str())
        };
        result = result
            .replace(&pattern_lower, " ")
            .replace(&pattern_upper, " ");
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
