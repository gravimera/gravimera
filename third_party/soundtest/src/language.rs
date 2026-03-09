use whatlang::{Lang, Script};

pub fn decide_language_code(text: &str) -> Option<&'static str> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    if let Some(info) = whatlang::detect(text) {
        if info.is_reliable() || info.confidence() >= 0.25 {
            return Some(iso639_1_for_whatlang(info.lang()));
        }
        return iso639_1_for_script(info.script());
    }

    whatlang::detect_script(text).and_then(iso639_1_for_script)
}

fn iso639_1_for_script(script: Script) -> Option<&'static str> {
    match script {
        Script::Arabic => Some("ar"),
        Script::Armenian => Some("hy"),
        Script::Bengali => Some("bn"),
        Script::Cyrillic => Some("ru"),
        Script::Devanagari => Some("hi"),
        Script::Ethiopic => Some("am"),
        Script::Georgian => Some("ka"),
        Script::Greek => Some("el"),
        Script::Gujarati => Some("gu"),
        Script::Gurmukhi => Some("pa"),
        Script::Hangul => Some("ko"),
        Script::Hebrew => Some("he"),
        Script::Hiragana | Script::Katakana => Some("ja"),
        Script::Kannada => Some("kn"),
        Script::Khmer => Some("km"),
        Script::Malayalam => Some("ml"),
        Script::Mandarin => Some("zh"),
        Script::Myanmar => Some("my"),
        Script::Oriya => Some("or"),
        Script::Sinhala => Some("si"),
        Script::Tamil => Some("ta"),
        Script::Telugu => Some("te"),
        Script::Thai => Some("th"),
        Script::Latin => None,
    }
}

fn iso639_1_for_whatlang(lang: Lang) -> &'static str {
    match lang {
        Lang::Aka => "ak",
        Lang::Afr => "af",
        Lang::Amh => "am",
        Lang::Ara => "ar",
        Lang::Aze => "az",
        Lang::Bel => "be",
        Lang::Ben => "bn",
        Lang::Bul => "bg",
        Lang::Cat => "ca",
        Lang::Ces => "cs",
        Lang::Cmn => "zh",
        Lang::Cym => "cy",
        Lang::Dan => "da",
        Lang::Deu => "de",
        Lang::Ell => "el",
        Lang::Eng => "en",
        Lang::Epo => "eo",
        Lang::Est => "et",
        Lang::Fin => "fi",
        Lang::Fra => "fr",
        Lang::Guj => "gu",
        Lang::Heb => "he",
        Lang::Hin => "hi",
        Lang::Hrv => "hr",
        Lang::Hun => "hu",
        Lang::Hye => "hy",
        Lang::Ind => "id",
        Lang::Ita => "it",
        Lang::Jav => "jv",
        Lang::Jpn => "ja",
        Lang::Kan => "kn",
        Lang::Kat => "ka",
        Lang::Khm => "km",
        Lang::Kor => "ko",
        Lang::Lat => "la",
        Lang::Lav => "lv",
        Lang::Lit => "lt",
        Lang::Mal => "ml",
        Lang::Mar => "mr",
        Lang::Mkd => "mk",
        Lang::Mya => "my",
        Lang::Nep => "ne",
        Lang::Nld => "nl",
        Lang::Nob => "nb",
        Lang::Ori => "or",
        Lang::Pan => "pa",
        Lang::Pes => "fa",
        Lang::Pol => "pl",
        Lang::Por => "pt",
        Lang::Ron => "ro",
        Lang::Rus => "ru",
        Lang::Sin => "si",
        Lang::Slk => "sk",
        Lang::Slv => "sl",
        Lang::Sna => "sn",
        Lang::Spa => "es",
        Lang::Srp => "sr",
        Lang::Swe => "sv",
        Lang::Tam => "ta",
        Lang::Tel => "te",
        Lang::Tgl => "tl",
        Lang::Tha => "th",
        Lang::Tuk => "tk",
        Lang::Tur => "tr",
        Lang::Ukr => "uk",
        Lang::Urd => "ur",
        Lang::Uzb => "uz",
        Lang::Vie => "vi",
        Lang::Yid => "yi",
        Lang::Zul => "zu",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decides_common_languages() {
        let en = "Hello! My name is Alex and I like building games with audio.";
        assert_eq!(decide_language_code(en), Some("en"));
        let fr = "Bonjour ! Je m'appelle Thomas et j'adore créer des jeux avec du son.";
        assert_eq!(decide_language_code(fr), Some("fr"));
        let es = "¡Hola! Me llamo Mónica y me encanta crear juegos con sonido y música.";
        assert_eq!(decide_language_code(es), Some("es"));
        assert_eq!(decide_language_code("Привет, как дела?"), Some("ru"));
        assert_eq!(decide_language_code("Γειά σου, τι κάνεις;"), Some("el"));
    }

    #[test]
    fn decides_non_latin_scripts() {
        assert_eq!(decide_language_code("恭喜发财！"), Some("zh"));
        assert_eq!(decide_language_code("こんにちは、元気ですか？"), Some("ja"));
        assert_eq!(decide_language_code("안녕하세요, 잘 지내요?"), Some("ko"));
        assert_eq!(decide_language_code("مرحبا كيف حالك"), Some("ar"));
        assert_eq!(decide_language_code("שלום מה שלומך"), Some("he"));
        assert_eq!(decide_language_code("नमस्ते आप कैसे हैं"), Some("hi"));
        assert_eq!(decide_language_code("สวัสดี คุณเป็นอย่างไรบ้าง"), Some("th"));
    }
}
