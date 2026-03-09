use crate::effects::EffectSpec;
use anyhow::{Result, anyhow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Onnx,
    System,
    Procedural,
}

#[derive(Debug, Clone)]
pub struct RenderPlan {
    pub backend: BackendKind,
    pub text: Option<String>,
    pub proc: Option<String>,
    pub effects: EffectSpec,
    pub raw: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveField {
    None,
    Text,
    Proc,
}

impl RenderPlan {
    pub fn parse(text: &str) -> Result<Self> {
        let mut backend_raw: Option<String> = None;
        let mut text_field: Option<String> = None;
        let mut proc_field: Option<String> = None;
        let mut effects = EffectSpec::default();
        let mut active = ActiveField::None;

        for raw_line in text.lines() {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("backend:") {
                backend_raw = Some(value.trim().to_ascii_lowercase());
                active = ActiveField::None;
                continue;
            }

            if trimmed.starts_with("tts_backend:") {
                // Legacy field from older "combo" plans; ignore.
                active = ActiveField::None;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("text:") {
                text_field = Some(value.trim_start().to_owned());
                active = ActiveField::Text;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("tts:") {
                // Legacy synonym for `text:`.
                text_field = Some(value.trim_start().to_owned());
                active = ActiveField::Text;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("proc:") {
                proc_field = Some(value.trim_start().to_owned());
                active = ActiveField::Proc;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("preset:") {
                effects.preset = value.trim().to_ascii_lowercase();
                active = ActiveField::None;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("amount:") {
                if let Some(v) = parse_f32(value) {
                    effects.amount = v;
                }
                active = ActiveField::None;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("speed:") {
                effects.speed = parse_f32(value);
                active = ActiveField::None;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("pitch_semitones:") {
                effects.pitch_semitones = parse_f32(value);
                active = ActiveField::None;
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("pitch:") {
                effects.pitch_semitones = parse_f32(value);
                active = ActiveField::None;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("bass_db:") {
                effects.bass_db = parse_f32(value);
                active = ActiveField::None;
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("bass:") {
                effects.bass_db = parse_f32(value);
                active = ActiveField::None;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("treble_db:") {
                effects.treble_db = parse_f32(value);
                active = ActiveField::None;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("reverb:") {
                effects.reverb = parse_f32(value);
                active = ActiveField::None;
                continue;
            }

            if let Some(value) = trimmed.strip_prefix("distortion:") {
                effects.distortion = parse_f32(value);
                active = ActiveField::None;
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("dist:") {
                effects.distortion = parse_f32(value);
                active = ActiveField::None;
                continue;
            }

            match active {
                ActiveField::Text => {
                    if let Some(prev) = &mut text_field {
                        prev.push('\n');
                        prev.push_str(raw_line.trim_end());
                    }
                }
                ActiveField::Proc => {
                    if let Some(prev) = &mut proc_field {
                        prev.push('\n');
                        prev.push_str(trimmed);
                    }
                }
                ActiveField::None => {}
            }
        }

        let mut backend = match backend_raw.as_deref() {
            Some("combo") => None,
            Some(other) => Some(parse_backend(other)?),
            None => None,
        }
        .unwrap_or_else(|| infer_backend(text_field.as_deref(), proc_field.as_deref()));

        let has_text = text_field.as_deref().is_some_and(|t| !t.trim().is_empty());
        let has_proc = proc_field.as_deref().is_some_and(|p| !p.trim().is_empty());

        backend = match backend {
            BackendKind::Procedural if !has_proc && has_text => BackendKind::System,
            BackendKind::System | BackendKind::Onnx if !has_text && has_proc => BackendKind::Procedural,
            other => other,
        };

        // Enforce the "exactly one backend output kind" rule.
        match backend {
            BackendKind::Procedural => text_field = None,
            BackendKind::System | BackendKind::Onnx => proc_field = None,
        }

        Ok(Self {
            backend,
            text: text_field.map(|t| t.trim().to_owned()),
            proc: proc_field.map(|p| p.trim().to_owned()),
            effects,
            raw: text.trim().to_owned(),
        })
    }

    pub fn fallback_text(text: &str) -> Self {
        Self {
            backend: BackendKind::System,
            text: Some(text.trim().to_owned()),
            proc: None,
            effects: EffectSpec::default(),
            raw: text.trim().to_owned(),
        }
    }

    pub fn to_debug_string(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("backend: {:?}\n", self.backend));
        match self.backend {
            BackendKind::Procedural => {
                if let Some(proc) = &self.proc {
                    out.push_str(&format!("proc: {}\n", proc));
                }
            }
            BackendKind::System | BackendKind::Onnx => {
                if let Some(text) = &self.text {
                    out.push_str(&format!("text: {}\n", text));
                }
                out.push_str(&format!("preset: {}\n", self.effects.preset));
                out.push_str(&format!("amount: {}\n", self.effects.amount));
                if let Some(speed) = self.effects.speed {
                    out.push_str(&format!("speed: {}\n", speed));
                }
                if let Some(pitch) = self.effects.pitch_semitones {
                    out.push_str(&format!("pitch_semitones: {}\n", pitch));
                }
                if let Some(bass_db) = self.effects.bass_db {
                    out.push_str(&format!("bass_db: {}\n", bass_db));
                }
                if let Some(treble_db) = self.effects.treble_db {
                    out.push_str(&format!("treble_db: {}\n", treble_db));
                }
                if let Some(reverb) = self.effects.reverb {
                    out.push_str(&format!("reverb: {}\n", reverb));
                }
                if let Some(dist) = self.effects.distortion {
                    out.push_str(&format!("distortion: {}\n", dist));
                }
            }
        }
        out
    }
}

fn parse_backend(value: &str) -> Result<BackendKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "onnx" | "chatterbox" | "chatterbox_onnx" | "local" => Ok(BackendKind::Onnx),
        "system" | "tts" => Ok(BackendKind::System),
        // Legacy/back-compat values.
        "piper" | "espeak" | "espeak-ng" | "espeakng" => Ok(BackendKind::System),
        "procedural" | "proc" => Ok(BackendKind::Procedural),
        other => Err(anyhow!("unknown backend `{}`", other)),
    }
}

fn infer_backend(text: Option<&str>, proc: Option<&str>) -> BackendKind {
    let has_text = text.is_some_and(|t| !t.trim().is_empty());
    let has_proc = proc.is_some_and(|p| !p.trim().is_empty());
    if has_proc && !has_text {
        return BackendKind::Procedural;
    }
    BackendKind::System
}

fn parse_f32(value: &str) -> Option<f32> {
    value.trim().parse::<f32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tts_text_plan() {
        let plan = RenderPlan::parse("backend: system\ntext: hello world").unwrap();
        assert_eq!(plan.backend, BackendKind::System);
        assert_eq!(plan.text.as_deref(), Some("hello world"));
        assert!(plan.proc.is_none());
    }

    #[test]
    fn parses_onnx_text_plan() {
        let plan = RenderPlan::parse("backend: onnx\ntext: hello world").unwrap();
        assert_eq!(plan.backend, BackendKind::Onnx);
        assert_eq!(plan.text.as_deref(), Some("hello world"));
        assert!(plan.proc.is_none());
    }

    #[test]
    fn parses_legacy_tts_field() {
        let plan = RenderPlan::parse("backend: system\ntts: hello").unwrap();
        assert_eq!(plan.backend, BackendKind::System);
        assert_eq!(plan.text.as_deref(), Some("hello"));
    }

    #[test]
    fn parses_procedural_plan() {
        let plan = RenderPlan::parse("backend: procedural\nproc: wind...").unwrap();
        assert_eq!(plan.backend, BackendKind::Procedural);
        assert_eq!(plan.proc.as_deref(), Some("wind..."));
        assert!(plan.text.is_none());
    }

    #[test]
    fn parses_effect_fields() {
        let plan = RenderPlan::parse(
            "backend: system\ntext: hi\npreset: dragon\namount: 0.7\nspeed: 0.9\npitch_semitones: -4\nbass_db: 6\nreverb: 0.2\ndistortion: 0.1",
        )
        .unwrap();
        assert_eq!(plan.effects.preset, "dragon");
        assert!((plan.effects.amount - 0.7).abs() < 1e-6);
        assert_eq!(plan.effects.speed, Some(0.9));
        assert_eq!(plan.effects.pitch_semitones, Some(-4.0));
        assert_eq!(plan.effects.bass_db, Some(6.0));
        assert_eq!(plan.effects.reverb, Some(0.2));
        assert_eq!(plan.effects.distortion, Some(0.1));
    }
}
