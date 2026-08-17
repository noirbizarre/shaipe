//! Noticing that the project changed underneath the workspace.
//!
//! Shaipe reads the project once and then holds it in memory, which is fine
//! until something else writes the same file — you in another window, or an
//! agent reaching for its own editor instead of `write_svg`. Nothing over ACP
//! can stop an agent doing that: OpenCode's permissions default to `allow`, so
//! it never asks, and a client that is never asked cannot refuse. See
//! ADR 012.
//!
//! Left unnoticed, that diverges silently and then `ctrl-s` overwrites the
//! other writer's work with bytes read minutes ago.
//!
//! Polled rather than watched. The workspace already has a tick, `mtime` and
//! length are two fields of one `stat`, and the alternative is a
//! filesystem-notification dependency and a background thread for something
//! that has to be checked at most four times a second.

use std::path::Path;
use std::time::SystemTime;

/// What the file looked like when it was last read or written.
///
/// Length as well as `mtime`, because a filesystem whose timestamps have a
/// one-second granularity will happily report the same `mtime` for a write
/// that happened within it — and an SVG being edited is exactly the kind of
/// file that changes length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl Stamp {
    /// Read the file's current stamp.
    ///
    /// `None` when it cannot be read at all — deleted, or renamed out from
    /// under us. Deliberately not an error: a project whose file has gone is
    /// still perfectly usable in memory, and saving it will put it back.
    #[must_use]
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

/// Whether the file has changed since it was last read or written.
#[derive(Debug, Clone)]
pub struct Watcher {
    /// `None` if the file could not be read when the watch started, which
    /// makes every later reading a change and is the honest answer.
    seen: Option<Stamp>,
}

impl Watcher {
    /// Start watching a file as it is now.
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self {
            seen: Stamp::of(path),
        }
    }

    /// Take note of the file as it is now, without reporting a change.
    ///
    /// Called after the workspace itself writes, so its own save is never
    /// mistaken for somebody else's.
    pub fn accept(&mut self, path: &Path) {
        self.seen = Stamp::of(path);
    }

    /// Whether the file differs from the last state this watcher accepted.
    ///
    /// Reports `true` only once per change: the new stamp is taken as seen, so
    /// a workspace that decides to ignore a change is not asked about it again
    /// every tick.
    pub fn changed(&mut self, path: &Path) -> bool {
        let current = Stamp::of(path);
        if current == self.seen {
            return false;
        }

        self.seen = current;
        true
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// Write a file, and make sure the change is visible to `stat`.
    ///
    /// `mtime` granularity is a filesystem property — one second on some — so
    /// a test that wrote twice in the same instant would be testing the
    /// filesystem rather than this module. The length changes too, which is
    /// exactly the belt-and-braces the `Stamp` exists for.
    fn write(path: &Path, contents: &str) {
        std::fs::write(path, contents).expect("the file is writable");
    }

    #[test]
    fn a_file_nobody_touched_has_not_changed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        write(&path, "<svg/>");

        let mut watcher = Watcher::new(&path);
        assert!(!watcher.changed(&path));
        assert!(
            !watcher.changed(&path),
            "asking twice must not invent a change"
        );
    }

    #[test]
    fn a_write_by_somebody_else_is_noticed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        write(&path, "<svg/>");

        let mut watcher = Watcher::new(&path);
        write(&path, "<svg><circle/></svg>");

        assert!(watcher.changed(&path));
    }

    #[test]
    fn a_change_is_reported_once_and_not_every_tick() {
        // Otherwise a workspace that chose to keep its own edits would be
        // asked about the same write four times a second, forever.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        write(&path, "<svg/>");

        let mut watcher = Watcher::new(&path);
        write(&path, "<svg><circle/></svg>");

        assert!(watcher.changed(&path));
        assert!(!watcher.changed(&path));
    }

    #[test]
    fn a_write_the_workspace_made_itself_is_not_a_change() {
        // The workspace saves, then accepts. Without this every `ctrl-s` would
        // immediately report the file as modified by somebody else.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        write(&path, "<svg/>");

        let mut watcher = Watcher::new(&path);
        write(&path, "<svg><circle/></svg>");
        watcher.accept(&path);

        assert!(!watcher.changed(&path));
    }

    #[test]
    fn a_file_that_is_not_there_is_not_an_error() {
        // A project whose file has been renamed away is still usable in
        // memory, and saving it puts it back.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gone.svg");

        assert_eq!(Stamp::of(&path), None);

        let mut watcher = Watcher::new(&path);
        assert!(!watcher.changed(&path));

        write(&path, "<svg/>");
        assert!(watcher.changed(&path), "it coming back is a change");
    }

    #[test]
    fn a_change_of_length_alone_is_noticed() {
        // The belt to `mtime`'s braces: a filesystem with one-second timestamp
        // granularity reports the same `mtime` for two writes inside it.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        write(&path, "<svg/>");

        let mut watcher = Watcher::new(&path);
        let stamp = watcher.seen.expect("the file is there");

        // Same instant, different length.
        write(&path, "<svg><circle/></svg>");
        let after = Stamp::of(&path).unwrap();

        assert_ne!(stamp.len, after.len);
        assert!(watcher.changed(&path));
    }
}
