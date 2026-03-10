# Meta Speak (Soundtest Integration)

The Meta panel includes a **Speak** section that lets a player type text and play AI voice output using one of three voice presets:

- `dog`
- `cow`
- `dragon`

UI path:

1. Enter Build/Play rendered mode.
2. Double-click a unit selection circle to open **Meta**.
3. In **Speak**, select voice, click the `content` field, type text, then click `Speak`.

## Architecture (B + Adapter Isolation)

Gravimera vendors `soundtest` inside this repository at `third_party/soundtest` and uses an adapter layer so UI code does not depend on `soundtest` internals.

- Adapter module: `src/meta_speak.rs`
- UI wiring: `src/motion_ui.rs`
- Bubble command + rendering: `src/types.rs` (`ModelSpeechBubbleCommand`) and `src/ui.rs`

Core abstraction:

- `MetaSpeakAdapter` trait: one method `speak(request)`
- `SoundtestMetaSpeakAdapter`: default implementation backed by `soundtest`
- `MetaSpeakRuntime` resource: stores adapter as `Arc<dyn MetaSpeakAdapter>`

This keeps the Meta UI stable if we later swap the backend implementation.

## Speech Bubble Channel (Trigger-Agnostic)

Speech bubbles are not coupled to the Meta panel input widgets. The UI uses a command message channel:

- `ModelSpeechBubbleCommand::Start { entity, text, source }`
- `ModelSpeechBubbleCommand::Stop { entity }`

Current producer:

- Meta panel Speak button (`source = MetaUi`)

Planned producer:

- Network-triggered model speech (`source = Network`)

Because rendering only consumes the command channel, future trigger/content changes do not require refactoring bubble rendering logic.

When speech starts, a bubble appears centered above the speaking model with centered text and a small bottom tail. Horizontal anchoring uses the model center line while vertical anchoring uses the model top offset, so the bubble stays visually centered over the head. Positioning math normalizes UI node size into logical pixel space (matching viewport projection), avoiding consistent left/up drift on high-DPI displays. The bubble stays hidden until UI layout size stabilizes, so it does not flash or jump during first-frame/early-frame text reflow. When speech finishes/fails/stops, the bubble is removed.

## Backend Resolution

Speak uses offline flow (`no_ai`) and prefers backends in this order:

1. ONNX TTS (if available)
2. System TTS (if available)

If neither is available, the UI shows an inline error.

## Voice Mapping

Voice presets are mapped to `soundtest` effects:

- `dog` -> `preset=neutral` with slight high-pitch/fast tuning
- `cow` -> `preset=giant` with lower pitch/slower tuning
- `dragon` -> `preset=dragon`

## Input Behavior

- `content` field supports direct typing, `Backspace`, `Esc`, paste (`Ctrl/Cmd+V`), IME-based Chinese input, and emoji. The hint text only appears when the field is empty.
- While `content` is focused, gameplay keyboard state is suppressed so typing does not trigger movement/shortcuts.

## Build Dependency Notes

`gravimera` now depends on `soundtest` and pins `ort` to `2.0.0-rc.11` to match `soundtest` compatibility.
