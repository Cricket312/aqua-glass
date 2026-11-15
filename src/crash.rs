//! Crash reporting. Release builds abort on panic and have no console, so
//! without this a crash just makes the window vanish. Any panic (on any
//! thread) and, on Windows, any unhandled native exception are written to
//! `<data>/crash.log`, and a message box tells the user where it is.
//! The report contains no tokens (messages are scrubbed) and no settings.

use std::io::Write;

pub fn log_path() -> std::path::PathBuf {
    crate::util::data_dir().join("crash.log")
}

fn write_report(body: &str) {
    let path = log_path();
    let _ = crate::util::ensure_dir(&crate::util::data_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(
            f,
            "==== Aqua Glass {} crash at unix {} ({} {}) ====\n{}\n",
            env!("CARGO_PKG_VERSION"),
            crate::util::now_unix(),
            std::env::consts::OS,
            std::env::consts::ARCH,
            body
        );
    }
    log::error!("{body}");
}

#[cfg(windows)]
fn message_box(text: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let t: Vec<u16> = "Aqua Glass crashed".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(None, PCWSTR(w.as_ptr()), PCWSTR(t.as_ptr()), MB_OK | MB_ICONERROR);
    }
}

pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let msg = crate::security::scrub(&msg);
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let bt = std::backtrace::Backtrace::force_capture();
        write_report(&format!("panic in thread '{thread}' at {loc}:\n  {msg}\n\nbacktrace:\n{bt}"));
        eprintln!("Aqua Glass crashed: {msg} at {loc}");
        #[cfg(windows)]
        message_box(&format!(
            "Aqua Glass hit an internal error and has to close.\n\n{msg}\n({loc}, thread '{thread}')\n\nDetails were saved to:\n{}\n\nPlease send that file when reporting the problem. Starting with --safe-mode skips restoring your Spotify/YouTube sessions and queue.",
            log_path().display()
        ));
    }));
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Diagnostics::Debug::{SetUnhandledExceptionFilter, EXCEPTION_POINTERS};
        unsafe extern "system" fn filter(p: *const EXCEPTION_POINTERS) -> i32 {
            let (code, addr) = unsafe {
                p.as_ref()
                    .and_then(|e| e.ExceptionRecord.as_ref())
                    .map(|r| (r.ExceptionCode.0 as u32, r.ExceptionAddress as usize))
                    .unwrap_or((0, 0))
            };
            let what = match code {
                0xC0000005 => "access violation",
                0xC00000FD => "stack overflow",
                0xC0000409 => "stack buffer overrun / fail-fast",
                0xC000001D => "illegal instruction",
                _ => "native exception",
            };
            write_report(&format!("{what}: code 0x{code:08X} at address 0x{addr:X} (thread '{}')", std::thread::current().name().unwrap_or("unnamed")));
            0 // EXCEPTION_CONTINUE_SEARCH: let Windows report it as usual
        }
        SetUnhandledExceptionFilter(Some(filter));
    }
}
