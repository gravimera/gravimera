use gravimera::meta_speak::{MetaSpeakVoice, SoundtestMetaSpeakAdapter};
use soundtest::render_plan::BackendKind;

#[test]
fn meta_speak_voice_list_is_fixed() {
    let voices = MetaSpeakVoice::all();
    let ids: Vec<&str> = voices.iter().map(|voice| voice.id_str()).collect();
    assert_eq!(ids, vec!["dog", "cow", "dragon"]);
}

#[test]
fn meta_speak_voice_effect_presets_match_contract() {
    let dog = MetaSpeakVoice::Dog.effect_spec();
    let cow = MetaSpeakVoice::Cow.effect_spec();
    let dragon = MetaSpeakVoice::Dragon.effect_spec();

    assert_eq!(dog.preset, "neutral");
    assert_eq!(cow.preset, "giant");
    assert_eq!(dragon.preset, "dragon");
}

#[test]
fn meta_speak_backend_selection_prefers_onnx_then_system() {
    assert_eq!(
        SoundtestMetaSpeakAdapter::choose_tts_backend_for_tests(true, true),
        Some(BackendKind::Onnx)
    );
    assert_eq!(
        SoundtestMetaSpeakAdapter::choose_tts_backend_for_tests(false, true),
        Some(BackendKind::System)
    );
    assert_eq!(
        SoundtestMetaSpeakAdapter::choose_tts_backend_for_tests(false, false),
        None
    );
}
