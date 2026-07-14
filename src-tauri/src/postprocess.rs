use crate::config::Config;

const FILLERS: &[&str] = &[
    "um", "uh", "like", "you know", "actually", "basically",
    "literally", "sort of", "kind of", "i mean", "well",
];

pub fn postprocess(text: &str, config: &Config) -> String {
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
