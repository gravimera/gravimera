use std::process::Stdio;

pub(crate) fn read_text() -> Option<String> {
    imp::read_text()
}

pub(crate) fn write_text(text: &str) -> bool {
    imp::write_text(text)
}

fn decode_clipboard_stdout(stdout: &[u8]) -> Option<String> {
    if stdout.is_empty() {
        return None;
    }

    let has_nul = stdout.iter().any(|&b| b == 0);
    if has_nul && stdout.len() % 2 == 0 {
        let mut units = Vec::with_capacity(stdout.len() / 2);
        for chunk in stdout.chunks_exact(2) {
            units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        let mut text = String::from_utf16_lossy(&units);
        if text.starts_with('\u{feff}') {
            text = text.trim_start_matches('\u{feff}').to_string();
        }
        return (!text.trim().is_empty()).then_some(text);
    }

    let mut text = String::from_utf8_lossy(stdout).to_string();
    if text.starts_with('\u{feff}') {
        text = text.trim_start_matches('\u{feff}').to_string();
    }
    (!text.trim().is_empty()).then_some(text)
}

fn read_text_command(cmd: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    decode_clipboard_stdout(&output.stdout)
}

fn write_text_command(cmd: &str, args: &[&str], text: &str) -> bool {
    let mut child = match std::process::Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        if stdin.write_all(text.as_bytes()).is_err() {
            return false;
        }
    }

    child.wait().map(|s| s.success()).unwrap_or(false)
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{read_text_command, write_text_command};

    pub(super) fn read_text() -> Option<String> {
        read_text_command("pbpaste", &[])
    }

    pub(super) fn write_text(text: &str) -> bool {
        write_text_command("pbcopy", &[], text)
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::{read_text_command, write_text_command};

    fn read_text_powershell(cmd: &str) -> Option<String> {
        read_text_command(
            cmd,
            &[
                "-NoProfile",
                "-Command",
                "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; [Console]::Write((Get-Clipboard -Raw))",
            ],
        )
    }

    pub(super) fn read_text() -> Option<String> {
        read_text_powershell("powershell")
    }

    pub(super) fn write_text(text: &str) -> bool {
        write_text_command("clip", &[], text)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::{read_text_command, write_text_command};

    fn read_text_powershell(cmd: &str) -> Option<String> {
        read_text_command(
            cmd,
            &[
                "-NoProfile",
                "-Command",
                "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; [Console]::Write((Get-Clipboard -Raw))",
            ],
        )
    }

    fn read_wsl_windows_clipboard_text() -> Option<String> {
        for cmd in [
            "powershell.exe",
            "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe",
        ] {
            if let Some(text) = read_text_powershell(cmd) {
                return Some(text);
            }
        }
        None
    }

    fn write_wsl_windows_clipboard_text(text: &str) -> bool {
        for cmd in [
            "clip.exe",
            "/mnt/c/Windows/System32/clip.exe",
            "/mnt/c/Windows/system32/clip.exe",
        ] {
            if write_text_command(cmd, &[], text) {
                return true;
            }
        }
        false
    }

    pub(super) fn read_text() -> Option<String> {
        if crate::platform::is_wsl() {
            if let Some(text) = read_wsl_windows_clipboard_text() {
                return Some(text);
            }
        }

        if let Some(text) = read_text_command("wl-paste", &["-n"]) {
            return Some(text);
        }
        if let Some(text) = read_text_command("xclip", &["-selection", "clipboard", "-o"]) {
            return Some(text);
        }
        read_text_command("xsel", &["--clipboard", "--output"])
    }

    pub(super) fn write_text(text: &str) -> bool {
        if crate::platform::is_wsl() && write_wsl_windows_clipboard_text(text) {
            return true;
        }

        if write_text_command("wl-copy", &[], text) {
            return true;
        }
        if write_text_command("xclip", &["-selection", "clipboard"], text) {
            return true;
        }
        write_text_command("xsel", &["--clipboard", "--input"], text)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
mod imp {
    pub(super) fn read_text() -> Option<String> {
        None
    }

    pub(super) fn write_text(_text: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::decode_clipboard_stdout;

    #[test]
    fn decode_utf8_clipboard_text() {
        assert_eq!(decode_clipboard_stdout(b"hello"), Some("hello".to_string()));
        assert_eq!(decode_clipboard_stdout(b"  \n\t "), None);
    }

    #[test]
    fn decode_utf16le_clipboard_text() {
        let utf16le = [b'h', 0, b'i', 0, b'!', 0];
        assert_eq!(decode_clipboard_stdout(&utf16le), Some("hi!".to_string()));
    }
}
