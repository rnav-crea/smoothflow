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
5. Keep first-person perspective.\n\n\
6. Output ONLY the cleaned transcript lines. No explanations, no notes.";

pub fn postprocess(text: &str, config: &Config) -> String {
    if config.ai_enhance && !config.cleanup_model.is_empty() {
        cleanup_transcript(text, config)
            .unwrap_or_else(|e| {
                println!("ENHANCE ERROR: {e}");
                basic_cleanup(text, config)
            })
    } else {
        basic_cleanup(text, config)
    }
}

fn basic_cleanup(text: &str, config: &Config) -> String {
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
            ai_enhance: false,
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
    fn ai_enhance_without_model_falls_back() {
        let mut c = test_config();
        c.ai_enhance = true;
        c.cleanup_model = String::new();
        let result = postprocess("hello world", &c);
        assert_eq!(result, "hello world.");
    }

    #[test]
    fn basic_handles_multiple_fillers() {
        let c = test_config();
        let result = postprocess("um like i mean hello world", &c);
        assert_eq!(result, "hello world.");
    }
}
