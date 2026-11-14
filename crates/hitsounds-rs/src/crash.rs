//! Crash reports (D-75). Release builds abort on a panic and have no console, so a
//! panic on any thread used to close the app without a word. The hook appends the
//! panic to `crash.log` in the config folder, then says so in a dialog before the
//! process ends.

use std::{
    backtrace::Backtrace,
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::Write as _,
    panic::PanicHookInfo,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

const LOG: &str = "crash.log";
const OLD_LOG: &str = "crash.old.log";
/// Past this size `crash.log` becomes `crash.old.log` and a new one starts.
const ROTATE_BYTES: u64 = 256 * 1024;

/// Installs the hook. The default hook still runs after it, so dev builds print the
/// panic to the console as before.
pub fn install() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = message(info);
        let report = report(info, &message);
        let saved = save(&report);
        default(info);
        tell_user(&message, saved);
    }));
}

fn message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_owned())
}

fn report(info: &PanicHookInfo<'_>, message: &str) -> String {
    let thread = std::thread::current();
    let mut text = String::new();
    let _ = writeln!(
        text,
        "=== hitsounds-rs {} crashed, {} ===",
        env!("CARGO_PKG_VERSION"),
        local_time()
    );
    let _ = writeln!(text, "Thread: {}", thread.name().unwrap_or("(unnamed)"));
    let _ = writeln!(text, "Panic: {message}");
    if let Some(at) = info.location() {
        let _ = writeln!(text, "At: {}:{}:{}", at.file(), at.line(), at.column());
    }
    let _ = writeln!(text, "Backtrace:\n{}", Backtrace::force_capture());
    text
}

/// Appends `report` to the log and returns its path, or `None` if it could not be
/// written anywhere.
fn save(report: &str) -> Option<PathBuf> {
    let folders = crate::worker::config_folder()
        .into_iter()
        .chain(std::iter::once(std::env::temp_dir()));
    for dir in folders {
        if fs::create_dir_all(&dir).is_err() {
            continue;
        }
        let path = dir.join(LOG);
        if fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_BYTES) {
            let _ = fs::rename(&path, dir.join(OLD_LOG));
        }
        let written = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| writeln!(file, "{report}"));
        if written.is_ok() {
            return Some(path);
        }
    }
    None
}

fn tell_user(message: &str, saved: Option<PathBuf>) {
    // Two threads panicking together would otherwise stack two dialogs.
    static SHOWN: AtomicBool = AtomicBool::new(false);
    if SHOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let ending = if cfg!(panic = "abort") {
        "hitsounds-rs ran into a bug and has to close."
    } else {
        "hitsounds-rs ran into a bug. This dev build keeps running, but may misbehave."
    };
    let mut text = format!("{ending}\n\n{message}\n\n");
    let dialog = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("hitsounds-rs crashed");
    let Some(path) = saved else {
        text.push_str("The crash log could not be written.");
        dialog.set_description(text).show();
        return;
    };
    let _ = write!(
        text,
        "The details are in {}. Include that file when you report this.\n\nShow it in File Explorer?",
        path.display()
    );
    let answer = dialog
        .set_description(text)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    if answer == rfd::MessageDialogResult::Yes {
        // Explorer is its own process, so it stays open after this one aborts.
        let _ = std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
}

fn local_time() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: `GetLocalTime` only writes the SYSTEMTIME it is given.
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}
