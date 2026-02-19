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
    use super::{decode_clipboard_stdout, read_text_command, write_text_command};

    #[cfg(target_os = "linux")]
    use std::sync::{mpsc, OnceLock};
    #[cfg(target_os = "linux")]
    use std::time::{Duration, Instant};
    #[cfg(target_os = "linux")]
    use x11rb::connection::Connection as _;
    #[cfg(target_os = "linux")]
    use x11rb::protocol::xproto::ConnectionExt as _;

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

    #[cfg(target_os = "linux")]
    fn x11_available() -> bool {
        std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
    }

    #[cfg(target_os = "linux")]
    fn x11_intern_atom<C: x11rb::connection::Connection>(conn: &C, name: &[u8]) -> Option<u32> {
        conn.intern_atom(false, name)
            .ok()?
            .reply()
            .ok()
            .map(|r| r.atom)
    }

    #[cfg(target_os = "linux")]
    fn x11_poll_until_selection_notify<C: x11rb::connection::Connection>(
        conn: &C,
        requestor: u32,
        selection: u32,
        deadline: Instant,
    ) -> Option<x11rb::protocol::xproto::SelectionNotifyEvent> {
        use x11rb::protocol::Event;

        while Instant::now() <= deadline {
            match conn.poll_for_event() {
                Ok(Some(Event::SelectionNotify(ev)))
                    if ev.requestor == requestor && ev.selection == selection =>
                {
                    return Some(ev);
                }
                Ok(Some(_)) => {}
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => return None,
            }
        }
        None
    }

    #[cfg(target_os = "linux")]
    fn x11_poll_until_property_new_value<C: x11rb::connection::Connection>(
        conn: &C,
        window: u32,
        atom: u32,
        deadline: Instant,
    ) -> Option<()> {
        use x11rb::protocol::xproto::Property;
        use x11rb::protocol::Event;

        while Instant::now() <= deadline {
            match conn.poll_for_event() {
                Ok(Some(Event::PropertyNotify(ev)))
                    if ev.window == window
                        && ev.atom == atom
                        && ev.state == Property::NEW_VALUE =>
                {
                    return Some(());
                }
                Ok(Some(_)) => {}
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => return None,
            }
        }
        None
    }

    #[cfg(target_os = "linux")]
    fn x11_read_property_bytes<C: x11rb::connection::Connection>(
        conn: &C,
        window: u32,
        property: u32,
        incr: u32,
        deadline: Instant,
    ) -> Option<Vec<u8>> {
        use x11rb::protocol::xproto::AtomEnum;

        let reply = conn
            .get_property(false, window, property, AtomEnum::ANY, 0, 1 << 20)
            .ok()?
            .reply()
            .ok()?;

        if reply.type_ != incr {
            return Some(reply.value);
        }

        // INCR transfer: delete property to signal readiness, then read chunks until the owner
        // sends a zero-length payload.
        conn.delete_property(window, property).ok()?;
        conn.flush().ok()?;

        let mut data = Vec::new();
        loop {
            x11_poll_until_property_new_value(conn, window, property, deadline)?;

            let chunk = conn
                .get_property(false, window, property, AtomEnum::ANY, 0, 1 << 20)
                .ok()?
                .reply()
                .ok()?;

            if chunk.value.is_empty() {
                break;
            }
            data.extend_from_slice(&chunk.value);
            conn.delete_property(window, property).ok()?;
            conn.flush().ok()?;
        }

        Some(data)
    }

    #[cfg(target_os = "linux")]
    fn x11_convert_selection_and_read_text<C: x11rb::connection::Connection>(
        conn: &C,
        window: u32,
        selection: u32,
        target: u32,
        property: u32,
        incr: u32,
        deadline: Instant,
    ) -> Option<String> {
        conn.convert_selection(window, selection, target, property, x11rb::CURRENT_TIME)
            .ok()?;
        conn.flush().ok()?;

        let ev = x11_poll_until_selection_notify(conn, window, selection, deadline)?;
        if ev.property == x11rb::NONE {
            return None;
        }

        let bytes = x11_read_property_bytes(conn, window, property, incr, deadline)?;
        decode_clipboard_stdout(&bytes)
    }

    #[cfg(target_os = "linux")]
    fn read_text_x11_clipboard() -> Option<String> {
        use x11rb::protocol::xproto::{AtomEnum, CreateWindowAux, EventMask, WindowClass};

        if !x11_available() {
            return None;
        }

        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let screen = conn.setup().roots.get(screen_num)?;

        let window = conn.generate_id().ok()?;
        let aux = CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE);
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            screen.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &aux,
        )
        .ok()?;

        let clipboard = x11_intern_atom(&conn, b"CLIPBOARD")?;
        let utf8 = x11_intern_atom(&conn, b"UTF8_STRING")?;
        let incr = x11_intern_atom(&conn, b"INCR")?;
        let property = x11_intern_atom(&conn, b"GRAVIMERA_X11_CLIPBOARD")?;

        let deadline = Instant::now() + Duration::from_millis(350);
        let text = x11_convert_selection_and_read_text(
            &conn, window, clipboard, utf8, property, incr, deadline,
        )
        .or_else(|| {
            x11_convert_selection_and_read_text(
                &conn,
                window,
                clipboard,
                AtomEnum::STRING.into(),
                property,
                incr,
                deadline,
            )
        });

        let _ = conn.destroy_window(window);
        let _ = conn.flush();
        text
    }

    #[cfg(target_os = "linux")]
    #[derive(Clone)]
    struct X11ClipboardAtoms {
        clipboard: u32,
        targets: u32,
        utf8: u32,
        text: u32,
        string: u32,
    }

    #[cfg(target_os = "linux")]
    fn x11_supported_targets(atoms: &X11ClipboardAtoms) -> Vec<u32> {
        vec![atoms.targets, atoms.utf8, atoms.text, atoms.string]
    }

    #[cfg(target_os = "linux")]
    fn x11_owner_setup<C: x11rb::connection::Connection>(
        conn: &C,
        screen_num: usize,
    ) -> Result<(u32, X11ClipboardAtoms), String> {
        use x11rb::protocol::xproto::{CreateWindowAux, EventMask, WindowClass};

        if !x11_available() {
            return Err("DISPLAY not set".into());
        }

        let setup = conn.setup();
        let screen = setup
            .roots
            .get(screen_num)
            .ok_or_else(|| "X11 setup missing root screen".to_string())?;

        let window = conn
            .generate_id()
            .map_err(|_| "Failed to generate X11 window id".to_string())?;
        let aux = CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE);
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            screen.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &aux,
        )
        .map_err(|_| "Failed to create X11 clipboard window".to_string())?;

        let clipboard = x11_intern_atom(conn, b"CLIPBOARD")
            .ok_or_else(|| "Failed to intern CLIPBOARD atom".to_string())?;
        let targets = x11_intern_atom(conn, b"TARGETS")
            .ok_or_else(|| "Failed to intern TARGETS atom".to_string())?;
        let utf8 = x11_intern_atom(conn, b"UTF8_STRING")
            .ok_or_else(|| "Failed to intern UTF8_STRING atom".to_string())?;
        let text = x11_intern_atom(conn, b"TEXT").unwrap_or(utf8);
        let string = x11rb::protocol::xproto::AtomEnum::STRING.into();

        Ok((
            window,
            X11ClipboardAtoms {
                clipboard,
                targets,
                utf8,
                text,
                string,
            },
        ))
    }

    #[cfg(target_os = "linux")]
    enum X11ClipboardCommand {
        SetText(String),
    }

    #[cfg(target_os = "linux")]
    static X11_CLIPBOARD_OWNER: OnceLock<mpsc::Sender<X11ClipboardCommand>> = OnceLock::new();

    #[cfg(target_os = "linux")]
    fn ensure_x11_clipboard_owner() -> Option<&'static mpsc::Sender<X11ClipboardCommand>> {
        if let Some(sender) = X11_CLIPBOARD_OWNER.get() {
            return Some(sender);
        }

        if !x11_available() {
            return None;
        }

        let (tx, rx) = mpsc::channel::<X11ClipboardCommand>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        std::thread::spawn(move || {
            use x11rb::connection::Connection;
            use x11rb::protocol::xproto::{
                self, AtomEnum, ConnectionExt as _, PropMode, SelectionNotifyEvent,
            };
            use x11rb::protocol::Event;
            use x11rb::wrapper::ConnectionExt as _;

            let run = (|| -> Result<(), String> {
                let (conn, screen_num) =
                    x11rb::connect(None).map_err(|err| format!("X11 connect failed: {err}"))?;
                let (window, atoms) = x11_owner_setup(&conn, screen_num)?;

                ready_tx.send(Ok(())).ok();

                let mut current_text = String::new();
                let mut owns_clipboard = false;

                loop {
                    while let Ok(cmd) = rx.try_recv() {
                        match cmd {
                            X11ClipboardCommand::SetText(text) => {
                                current_text = text;
                                // Claim clipboard ownership.
                                if conn
                                    .set_selection_owner(
                                        window,
                                        atoms.clipboard,
                                        x11rb::CURRENT_TIME,
                                    )
                                    .is_ok()
                                {
                                    owns_clipboard = true;
                                }
                                let _ = conn.flush();
                            }
                        }
                    }

                    match conn.poll_for_event() {
                        Ok(Some(Event::SelectionRequest(req))) => {
                            if req.selection != atoms.clipboard {
                                continue;
                            }

                            let property = if req.property == x11rb::NONE {
                                req.target
                            } else {
                                req.property
                            };

                            let mut notify_property = property;

                            if req.target == atoms.targets {
                                let mut supported = x11_supported_targets(&atoms);
                                supported.sort_unstable();
                                let _ = conn.change_property32(
                                    PropMode::REPLACE,
                                    req.requestor,
                                    property,
                                    AtomEnum::ATOM,
                                    &supported,
                                );
                            } else if req.target == atoms.utf8 || req.target == atoms.text {
                                let _ = conn.change_property8(
                                    PropMode::REPLACE,
                                    req.requestor,
                                    property,
                                    atoms.utf8,
                                    current_text.as_bytes(),
                                );
                            } else if req.target == atoms.string {
                                let _ = conn.change_property8(
                                    PropMode::REPLACE,
                                    req.requestor,
                                    property,
                                    atoms.string,
                                    current_text.as_bytes(),
                                );
                            } else {
                                notify_property = x11rb::NONE;
                            }

                            let notify = SelectionNotifyEvent {
                                response_type: xproto::SELECTION_NOTIFY_EVENT,
                                sequence: 0,
                                time: req.time,
                                requestor: req.requestor,
                                selection: req.selection,
                                target: req.target,
                                property: notify_property,
                            };
                            let _ = conn.send_event(
                                false,
                                req.requestor,
                                xproto::EventMask::NO_EVENT,
                                notify,
                            );
                            let _ = conn.flush();
                        }
                        Ok(Some(Event::SelectionClear(ev))) => {
                            if ev.selection == atoms.clipboard {
                                owns_clipboard = false;
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            // No X11 events. Wait briefly for new clipboard data to avoid busy-looping,
                            // but keep a low timeout so we can still respond to selection requests.
                            match rx.recv_timeout(Duration::from_millis(10)) {
                                Ok(X11ClipboardCommand::SetText(text)) => {
                                    current_text = text;
                                    if conn
                                        .set_selection_owner(
                                            window,
                                            atoms.clipboard,
                                            x11rb::CURRENT_TIME,
                                        )
                                        .is_ok()
                                    {
                                        owns_clipboard = true;
                                    }
                                    let _ = conn.flush();
                                }
                                Err(mpsc::RecvTimeoutError::Timeout) => {}
                                Err(mpsc::RecvTimeoutError::Disconnected) => {
                                    // If the sender is gone and we don't own the clipboard anymore,
                                    // we can stop the thread.
                                    if !owns_clipboard {
                                        break;
                                    }
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }

                let _ = conn.destroy_window(window);
                let _ = conn.flush();
                Ok(())
            })();

            // If setup failed before sending ready, try to report it.
            if ready_tx.send(run.map(|_| ())).is_ok() {
                // The main thread will ignore any messages after the first.
            }
        });

        match ready_rx.recv_timeout(Duration::from_millis(250)).ok()? {
            Ok(()) => {
                let _ = X11_CLIPBOARD_OWNER.set(tx);
                X11_CLIPBOARD_OWNER.get()
            }
            Err(_) => None,
        }
    }

    #[cfg(target_os = "linux")]
    fn write_text_x11_clipboard(text: &str) -> bool {
        let Some(sender) = ensure_x11_clipboard_owner() else {
            return false;
        };
        sender
            .send(X11ClipboardCommand::SetText(text.to_string()))
            .is_ok()
    }

    pub(super) fn read_text() -> Option<String> {
        if crate::platform::is_wsl() {
            if let Some(text) = read_wsl_windows_clipboard_text() {
                return Some(text);
            }
        }

        #[cfg(target_os = "linux")]
        if let Some(text) = read_text_x11_clipboard() {
            return Some(text);
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

        #[cfg(target_os = "linux")]
        if write_text_x11_clipboard(text) {
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
    #[cfg(target_os = "linux")]
    use std::time::{Duration, Instant};

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

    #[test]
    #[cfg(target_os = "linux")]
    fn x11_clipboard_round_trip_when_enabled() {
        if std::env::var_os("GRAVIMERA_TEST_CLIPBOARD").is_none() {
            return;
        }
        if std::env::var_os("DISPLAY").is_none() {
            return;
        }

        let payload = format!("gravimera-clipboard-test-{}", std::process::id());
        assert!(super::write_text(&payload));

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(readback) = super::read_text() {
                if readback == payload {
                    break;
                }
            }
            if Instant::now() >= deadline {
                panic!("clipboard round-trip failed");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
