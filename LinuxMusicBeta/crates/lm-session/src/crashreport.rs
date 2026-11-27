//! The report of a Rust panic (F-GER-004), on every platform: `logs/crash-report.txt` of the data folder, with the
//! time, the program's version, the thread, the place, the message and the backtrace. The next start of a session
//! says there is one (a notice) and moves it to `crash-report.last.txt`. Native crashes of a plugin are the
//! safe mode's business (`crashnote`).

use std::path::{Path, PathBuf};

/// The report's file in a logs folder
pub fn report_file(logs: &Path) -> PathBuf {
    logs.join("crash-report.txt")
}

/// The text of a report
pub fn report_text(info: &std::panic::PanicHookInfo<'_>, backtrace: &std::backtrace::Backtrace) -> String {
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_owned());
    let place = info.location().map_or_else(|| "?".to_owned(), |l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
    format!(
        "LinuxMusicBeta {}\ntime (seconds since the epoch): {}\nthread: {}\nat: {place}\nmessage: {message}\n\nbacktrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        crate::lock::now_seconds(),
        std::thread::current().name().unwrap_or("(unnamed)"),
    )
}

/// From now on a panic writes the report in `logs`, then goes on to the hook that was there (the message on the
/// standard error). The audio thread never panics by rule; this is for everything else
pub fn install(logs: PathBuf) {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = report_text(info, &std::backtrace::Backtrace::force_capture());
        let _ = std::fs::create_dir_all(&logs);
        let _ = std::fs::write(report_file(&logs), text);
        before(info);
    }));
}

/// A report left by the last run: moved aside so it is told once. `Some(path)` is where it is now
pub fn take_previous(logs: &Path) -> Option<PathBuf> {
    let file = report_file(logs);
    if !file.is_file() {
        return None;
    }
    let kept = logs.join("crash-report.last.txt");
    std::fs::rename(&file, &kept).ok().map(|()| kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_previous_report_is_told_once() {
        let dir = std::env::temp_dir().join(format!("lm-crash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(take_previous(&dir), None);
        std::fs::write(report_file(&dir), "x").unwrap();
        let kept = take_previous(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(kept).unwrap(), "x");
        assert_eq!(take_previous(&dir), None, "moved aside: the next start does not tell it again");
        std::fs::remove_dir_all(dir).ok();
    }
}
