# Fix Gen3D clipboard paste on WSL

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` (repo root).

## Purpose / Big Picture

When running Gravimera inside WSL on Windows, the Gen3D prompt box should accept paste via `Ctrl+V` (and Gen3D Tool Feedback “Copy …” buttons should work) without requiring the user to install Linux clipboard helper binaries (`wl-paste` / `wl-copy` / `xclip` / `xsel`). Users should be able to copy text in a Windows app, switch back to the game, paste into Gen3D, and proceed normally.

The primary mechanism will be WSL “Windows interop” clipboard commands (`powershell.exe Get-Clipboard -Raw` for reading and `clip.exe` for writing). If interop is unavailable, the game should fall back to the existing Linux command-based clipboard backends.

## Progress

- [x] (2026-02-10 13:20Z) Add `src/platform.rs` with reusable WSL detection and tests.
- [x] (2026-02-10 13:20Z) Add `src/clipboard.rs` implementing read/write with WSL-first backend + fallbacks.
- [x] (2026-02-10 13:20Z) Update `src/gen3d/ui.rs` to use `crate::clipboard` for prompt paste.
- [x] (2026-02-10 13:20Z) Update `src/gen3d/tool_feedback_ui.rs` to use `crate::clipboard` for copy buttons.
- [x] (2026-02-10 13:20Z) Update docs (`README.md`, `gen_3d.md`) to document WSL clipboard behavior.
- [x] (2026-02-10 13:20Z) Run `cargo test` and a headless startup smoke run.
- [ ] (2026-02-10) Commit with a clear message.

## Surprises & Discoveries

- Observation: Gen3D paste currently shells out to Linux clipboard tools only (`wl-paste` → `xclip` → `xsel`) and silently does nothing if none are available.
  Evidence: `src/gen3d/ui.rs` `read_clipboard_text()` returns `None` when commands are missing/failed and `gen3d_prompt_text_input()` has no fallback.
- Observation: On WSLg, Gravimera often forces winit to use X11 (unsets `WAYLAND_DISPLAY`) for stability. This can break a Wayland-only clipboard strategy (like `wl-paste`) even when WSLg is enabled.
  Evidence: `src/app.rs` `fixup_linux_display_env_for_winit()` sets `WINIT_UNIX_BACKEND=x11` and removes `WAYLAND_DISPLAY` under WSL when X11 is available.
- Observation: WSL Windows interop can be disabled, in which case attempting to execute Windows `.exe` binaries from WSL fails (often `Exec format error`). The clipboard code must treat this as a normal failure and fall back.
  Evidence: In this dev environment, `clip.exe` and `cmd.exe` are present on `/mnt/c` but cannot be executed from WSL.

## Decision Log

- Decision: Keep the project’s “use OS commands” approach and add a WSL-first backend that reads/writes the Windows clipboard via `powershell.exe` / `clip.exe`, while preserving existing Linux fallbacks.
  Rationale: This fixes WSL without new Rust dependencies and matches existing clipboard code in Gen3D (command execution). It also avoids relying on Wayland clipboard tools when the game forces X11 on WSLg.
  Date/Author: 2026-02-10 / assistant
- Decision: Decode clipboard command output as UTF-16LE when it contains NUL bytes and has an even length.
  Rationale: PowerShell clipboard output can surface as UTF-16LE in some interop contexts; decoding it makes paste robust for non-ASCII text.
  Date/Author: 2026-02-10 / assistant

## Outcomes & Retrospective

Gen3D prompt paste and Tool Feedback copy were moved to a shared clipboard module with a WSL-first backend. The game now prefers Windows clipboard interop on WSL (when available) and falls back to Linux clipboard helpers otherwise. Unit tests, integration tests, and a headless startup smoke run all pass.

Remaining limitation: if WSL interop is disabled and no Linux clipboard helper binaries are installed, clipboard operations will still be unavailable; the docs now describe the fallback options.

## Context and Orientation

The relevant UI behaviors live in:

`src/gen3d/ui.rs`

- `gen3d_prompt_text_input()` handles keyboard typing in the Gen3D prompt box.
- For `Ctrl/Cmd+V`, it currently calls `read_clipboard_text()` which uses OS commands (not any native clipboard API).

`src/gen3d/tool_feedback_ui.rs`

- Tool Feedback “Copy …” buttons currently write to the clipboard by shelling out to OS commands.

`src/app.rs`

- Contains WSL detection logic (currently private inside a Linux-only env “fixup” function).

The bug report (“cannot paste into Gen3D prompt box on WSL”) is consistent with WSL systems that do not have `wl-clipboard` / `xclip` / `xsel` installed, and with WSLg runs where the game forces X11 and unsets `WAYLAND_DISPLAY` (so `wl-paste` cannot succeed).

## Plan of Work

First, factor WSL detection into a reusable `crate::platform::is_wsl()` helper, so it can be used by both the display-env fixups and the clipboard implementation.

Then implement a shared clipboard module, `crate::clipboard`, with two entry points:

- `read_text() -> Option<String>` (clipboard read, best-effort)
- `write_text(text: &str) -> bool` (clipboard write, best-effort)

On Linux builds, `read_text()` and `write_text()` should:

1. If running in WSL, attempt to use Windows clipboard interop first:
   - Read: `powershell.exe -NoProfile -Command <Get-Clipboard -Raw>`
   - Write: pipe text to `clip.exe`
2. If that fails (interop disabled, command not found, non-zero status), fall back to the existing Linux command backends:
   - Read: `wl-paste -n`, else `xclip -selection clipboard -o`, else `xsel --clipboard --output`
   - Write: `wl-copy`, else `xclip -selection clipboard`, else `xsel --clipboard --input`

On Windows and macOS builds, keep the existing OS-specific command approach (`powershell`/`clip` for Windows, `pbpaste`/`pbcopy` for macOS).

Finally, wire Gen3D paste and Tool Feedback copy to use `crate::clipboard`, and update docs to explain WSL behavior and fallbacks.

## Concrete Steps

From the repository root (`/home/flow/github/gravimera`):

1. Add `src/platform.rs` and export `platform::is_wsl()`. Update `src/app.rs` to call it.
2. Add `src/clipboard.rs` with `read_text()` and `write_text()`.
3. Replace clipboard helper functions in:
   - `src/gen3d/ui.rs` (prompt paste) and
   - `src/gen3d/tool_feedback_ui.rs` (copy buttons)
   with calls into `crate::clipboard`.
4. Update docs:
   - `README.md` (WSL section)
   - `gen_3d.md` (prompt paste note)
5. Format and validate:
   - `cargo fmt`
   - `cargo test`
   - `cargo run -- --headless --headless-seconds 10`
6. Commit changes:
   - `git commit -am "Fix Gen3D clipboard on WSL"` (or equivalent, ensuring new files are added).

## Validation and Acceptance

Automated validation:

- `cargo test` passes.
- The headless smoke run starts and exits without crashing:
  - `cargo run -- --headless --headless-seconds 10`

Manual acceptance on WSL (rendered mode):

1. Copy multi-line text in a Windows app (Notepad).
2. Start Gravimera in WSL: `cargo run`.
3. Enter Gen3D mode and click the prompt box (ensures it is focused).
4. Press `Ctrl+V`.
5. Expected: the copied text appears in the prompt, and no error is shown.

Manual acceptance for Tool Feedback copy:

1. In Gen3D, open the Tool Feedback tab and click “Copy …”.
2. Paste into a Windows app.
3. Expected: the payload appears (proves clipboard write path works on WSL).

## Idempotence and Recovery

This change is safe to iterate on. Clipboard commands are executed only when the user explicitly pastes or clicks copy, so failures should degrade gracefully (no crashes). If the WSL-first interop backend causes regressions, it can be disabled by removing only the WSL branch and leaving the existing Linux fallbacks intact.
