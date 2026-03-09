use std::collections::HashMap;

pub fn sanitize_tts_text(input: &str) -> String {
    let lowered = input.to_ascii_lowercase();
    let hyphen_fixed = replace_letter_hyphens(&lowered);

    let mapping = default_mapping();

    let mut out = String::with_capacity(hyphen_fixed.len());
    let mut current_word = String::new();

    for ch in hyphen_fixed.chars() {
        if ch.is_ascii_alphabetic() {
            current_word.push(ch);
            continue;
        }

        flush_word(&mut out, &mut current_word, &mapping);

        match ch {
            '.' | ',' | '?' | '!' => out.push(ch),
            '\n' => out.push('\n'),
            _ => out.push(' '),
        }
    }

    flush_word(&mut out, &mut current_word, &mapping);

    normalize_spacing(&out)
}

fn default_mapping() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("mrrp", "murrp"),
        ("mrr", "murr"),
        ("prrr", "purr"),
        ("prrrr", "purrr"),
        ("grrr", "gurr"),
        ("grrrr", "gurrr"),
        ("arf", "woof"),
        ("bzzt", "buzz"),
        ("bzz", "buzz"),
        ("tkkk", "tukku"),
    ])
}

fn replace_letter_hyphens(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    for (i, ch) in chars.iter().enumerate() {
        if *ch != '-' {
            out.push(*ch);
            continue;
        }

        let prev_is_letter = i > 0 && chars[i - 1].is_ascii_alphabetic();
        let next_is_letter = i + 1 < chars.len() && chars[i + 1].is_ascii_alphabetic();
        if prev_is_letter && next_is_letter {
            out.push(',');
            out.push(' ');
        } else {
            out.push(' ');
        }
    }
    out
}

fn flush_word(out: &mut String, current_word: &mut String, mapping: &HashMap<&str, &str>) {
    if current_word.is_empty() {
        return;
    }

    let mut word = std::mem::take(current_word);
    if let Some(mapped) = mapping.get(word.as_str()) {
        word = mapped.to_string();
    }

    if !has_vowel(&word) {
        word = insert_vowel(&word);
    }

    out.push_str(&word);
}

fn has_vowel(word: &str) -> bool {
    word.chars()
        .any(|c| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y'))
}

fn insert_vowel(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let rest: String = chars.collect();
    format!("{first}u{rest}")
}

fn normalize_spacing(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_space = false;
    let mut prev_newline = false;

    for ch in input.chars() {
        if ch == '\n' {
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
            prev_space = false;
            prev_newline = true;
            continue;
        }

        if matches!(ch, ' ' | '\t' | '\r') {
            if prev_space || prev_newline {
                continue;
            }
            out.push(' ');
            prev_space = true;
            prev_newline = false;
            continue;
        }

        if matches!(ch, ',' | '.' | '?' | '!') {
            if out.ends_with(' ') {
                out.pop();
            }
            out.push(ch);
            prev_space = false;
            prev_newline = false;
            continue;
        }

        out.push(ch);
        prev_space = false;
        prev_newline = false;
    }

    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_acronym_like_tokens() {
        assert_eq!(sanitize_tts_text("Mrrp?"), "murrp?");
        assert_eq!(sanitize_tts_text("bzzt!"), "buzz!");
    }

    #[test]
    fn avoids_hyphenated_spelling() {
        assert_eq!(sanitize_tts_text("arf-arf..."), "woof, woof...");
    }
}
