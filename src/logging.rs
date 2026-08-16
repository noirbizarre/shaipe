//! Where warnings go.
//!
//! Shaipe emits very little: essentially, that a font fell back to the system
//! and the render is therefore no longer reproducible elsewhere. That is worth
//! a line on stderr and not worth a logging framework, so this is a `log`
//! implementation in thirty lines rather than a dependency.
//!
//! Warnings go to stderr, never stdout, because stdout carries the list of
//! written files and `shaipe inspect --format json` — both of which something
//! downstream is entitled to parse.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use log::{Level, LevelFilter, Metadata, Record};

/// Set while the alternate screen is in use.
///
/// A warning printed over a TUI corrupts the display and cannot be scrolled
/// back to, so during the workspace they are dropped rather than shown badly.
static SUPPRESSED: AtomicBool = AtomicBool::new(false);

/// The logger.
struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        !SUPPRESSED.load(Ordering::Relaxed)
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let label = match record.level() {
            Level::Error => "error",
            Level::Warn => "warning",
            Level::Info => "info",
            Level::Debug => "debug",
            Level::Trace => "trace",
        };
        // Failure to write a warning must not become a failure to render.
        let _ = writeln!(std::io::stderr(), "{label}: {}", record.args());
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

/// Install the logger.
///
/// `verbosity` is the number of times `-v` was given. Warnings are on from the
/// start, because the ones Shaipe emits are things the user needs to know
/// whether or not they asked.
///
/// Calling this more than once is harmless; the second call does nothing.
pub fn init(verbosity: u8) {
    let level = match verbosity {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    // `set_logger` fails only if one is already installed, which is not an
    // error worth propagating out of a logging setup call.
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(level);
    }
}

/// Stop emitting log records until the returned guard is dropped.
///
/// Held by the TUI for as long as it owns the screen.
#[must_use]
pub fn suppress() -> Suppressed {
    SUPPRESSED.store(true, Ordering::Relaxed);
    Suppressed
}

/// Restores logging when dropped.
///
/// A guard rather than a pair of calls so that an early return, or a panic
/// unwinding out of the workspace, cannot leave the process permanently
/// silent.
#[derive(Debug)]
#[non_exhaustive]
pub struct Suppressed;

impl Drop for Suppressed {
    fn drop(&mut self) {
        SUPPRESSED.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppression_ends_when_the_guard_is_dropped() {
        // If it did not, a font warning would be lost for the rest of the
        // process after the workspace had been opened once.
        {
            let _guard = suppress();
            assert!(SUPPRESSED.load(Ordering::Relaxed));
        }
        assert!(!SUPPRESSED.load(Ordering::Relaxed));
    }

    #[test]
    fn installing_the_logger_twice_is_harmless() {
        init(0);
        init(3);
    }
}
