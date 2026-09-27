//! Remembering which model was last chosen, for each agent Shaipe drives.
//!
//! Model choice itself is never Shaipe's to make — see ADR 004 — but *which*
//! model was last picked for a given agent command is Shaipe's own
//! preference to remember, the same way a terminal remembers its last
//! working directory. Without this, `--model`/`SHAIPE_MODEL` or the `M`
//! picker have to be repeated on every single run.
//!
//! See ADR 016 for why this lives in the OS config directory rather than
//! inside the project file: it is a preference about *how Shaipe is used*,
//! not about the artwork, and belongs nowhere near the one thing ADR 001
//! already declared the source of truth for.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The shape written to disk. Kept separate from [`Settings`] itself so that
/// `path` — meaningful only in memory, never something to serialise — can
/// never accidentally end up inside the file it names.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Document {
    /// An agent's command line -> the last model id chosen while driving it.
    ///
    /// Keyed by the command line rather than a single global preference: a
    /// model id is an agent's own vocabulary (see
    /// [`crate::acp::AgentConfig::with_model`]), and one that means something
    /// to `opencode` is not even guaranteed to parse for a different agent.
    #[serde(default)]
    models: BTreeMap<String, String>,
}

/// Where a chosen model was last remembered, and for which agent.
///
/// Tolerant by construction: a missing or corrupt file behaves exactly like
/// an empty one, and a failed write is swallowed rather than surfaced —
/// losing a remembered preference must never be worse than never having had
/// one. Nothing here is fatal to opening the workspace.
#[derive(Debug, Default)]
pub struct Settings {
    /// Where this was loaded from, and where a change is saved back to.
    ///
    /// Stored explicitly, rather than recomputed from a global default every
    /// time, so a test can point it at a private `tempdir` instead of the
    /// real, shared, machine-wide config directory — the same reason
    /// `src/fonts/mod.rs`'s `resolve_remote` takes its cache path as an
    /// explicit argument.
    path: PathBuf,
    models: BTreeMap<String, String>,
}

impl Settings {
    /// Load the remembered models from the real OS config directory.
    ///
    /// # Panics
    ///
    /// Never: an OS that cannot say where its config directory is falls back
    /// to a relative one, exactly as [`crate::fonts`] does for its cache.
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(default_path())
    }

    /// Load from an explicit path, without touching the real config
    /// directory. What lets tests exercise this without a global, shared,
    /// racing-under-`cargo test` location.
    #[must_use]
    fn load_from(path: PathBuf) -> Self {
        let models = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Document>(&bytes).ok())
            .map_or_else(BTreeMap::new, |document| document.models);

        Self { path, models }
    }

    /// The model last chosen for this exact agent command line, if any.
    #[must_use]
    pub fn model_for(&self, command: &str) -> Option<&str> {
        self.models.get(command).map(String::as_str)
    }

    /// Remember `model` as the one to use next time `command` is started.
    ///
    /// A no-op — no write, no I/O at all — when this is already what is
    /// remembered, so picking the model that is already current does not
    /// touch the file's modification time for nothing.
    ///
    /// Saving is best-effort: a failure (a read-only config directory, most
    /// likely) is not reported anywhere, because a preference that failed to
    /// save is not worse than one that was never asked for — the workspace
    /// keeps working either way, just without the reminder next time.
    pub fn set_model(&mut self, command: &str, model: &str) {
        if self.models.get(command).map(String::as_str) == Some(model) {
            return;
        }

        self.models.insert(command.to_owned(), model.to_owned());
        self.save();
    }

    /// Write the current state back to [`Self::path`], swallowing every
    /// failure. See [`Self::set_model`] for why.
    fn save(&self) {
        let Ok(json) = serde_json::to_vec_pretty(&Document {
            models: self.models.clone(),
        }) else {
            return;
        };

        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, json);
    }
}

/// `<config dir>/settings.json` — the same `ProjectDirs` qualifier
/// [`crate::fonts`] uses for its cache, `config_dir()` rather than
/// `cache_dir()`: a remembered preference is meant to persist, not to be
/// something a cache-clearing tool is expected to sweep away.
fn default_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "shaipe", "shaipe")
        // No project directories at all (an unusual, minimal environment) —
        // fall back to a directory relative to the process, so there is
        // still somewhere for a preference to land rather than one that can
        // never be remembered at all.
        .map_or_else(
            || PathBuf::from(".shaipe-cache"),
            |dirs| dirs.config_dir().to_path_buf(),
        )
        .join("settings.json")
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn a_missing_file_behaves_like_an_empty_one() {
        let directory = tempfile::tempdir().unwrap();
        let settings = Settings::load_from(directory.path().join("settings.json"));
        assert_eq!(settings.model_for("opencode"), None);
    }

    #[test]
    fn a_corrupt_file_behaves_like_an_empty_one_rather_than_failing_to_open() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        std::fs::write(&path, b"not json at all").unwrap();

        let settings = Settings::load_from(path);
        assert_eq!(settings.model_for("opencode"), None);
    }

    #[test]
    fn a_chosen_model_survives_a_reload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");

        let mut settings = Settings::load_from(path.clone());
        settings.set_model("opencode", "anthropic/claude-opus-4-5");

        let reloaded = Settings::load_from(path);
        assert_eq!(
            reloaded.model_for("opencode"),
            Some("anthropic/claude-opus-4-5")
        );
    }

    #[test]
    fn different_agent_commands_remember_different_models() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");

        let mut settings = Settings::load_from(path.clone());
        settings.set_model("opencode", "anthropic/claude-opus-4-5");
        settings.set_model("opencode acp", "opencode/grok-code");

        let reloaded = Settings::load_from(path);
        assert_eq!(
            reloaded.model_for("opencode"),
            Some("anthropic/claude-opus-4-5")
        );
        assert_eq!(
            reloaded.model_for("opencode acp"),
            Some("opencode/grok-code")
        );
    }

    #[test]
    fn setting_the_same_model_again_does_not_touch_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");

        let mut settings = Settings::load_from(path.clone());
        settings.set_model("opencode", "anthropic/claude-opus-4-5");
        let written_at = std::fs::metadata(&path).unwrap().modified().unwrap();

        // A file system's mtime resolution is coarse enough on some
        // platforms that two writes a microsecond apart look identical, so a
        // real write here would not necessarily prove anything by itself —
        // the point is that `set_model` returns before ever reaching
        // `save()` for an unchanged value, not that the clock moved.
        settings.set_model("opencode", "anthropic/claude-opus-4-5");
        let after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(written_at, after);
    }

    #[test]
    fn a_model_can_be_forgotten_for_one_agent_without_touching_another() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");

        let mut settings = Settings::load_from(path.clone());
        settings.set_model("opencode", "anthropic/claude-opus-4-5");
        settings.set_model("opencode", "anthropic/claude-haiku-4-5");

        let reloaded = Settings::load_from(path);
        assert_eq!(
            reloaded.model_for("opencode"),
            Some("anthropic/claude-haiku-4-5")
        );
    }
}
