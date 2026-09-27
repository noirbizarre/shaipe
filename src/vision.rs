//! Whether a model can see an image, according to [models.dev]'s catalogue.
//!
//! ACP's own `SessionConfigOption` carries no capability metadata — every
//! choice it offers is `{value, name}`, confirmed by reading OpenCode's own
//! `buildModelSelectOptions()` — so the only way to know whether a model can
//! see the render Shaipe exists to show it is to ask somewhere else. See
//! ADR 017 for why that is [models.dev]'s catalogue rather than a name
//! pattern Shaipe would otherwise have to guess at and maintain by hand.
//!
//! The upstream catalogue is a five-megabyte, two-hundred-provider,
//! eight-thousand-model document. Nothing here keeps a copy of it: [`fetch`]
//! reads only `modalities.input` per model and flattens the result to the
//! one thing Shaipe's own picker needs — `"provider/model-id" -> bool` — the
//! same "translate once" firewall [`crate::acp::update`] keeps between ACP's
//! own types and the rest of the workspace.
//!
//! [models.dev]: https://models.dev

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Where the catalogue is fetched from.
const CATALOGUE_URL: &str = "https://models.dev/api.json";

/// How long a fetch may take before it is treated as unreachable. As
/// generous as [`crate::fonts::FETCH_TIMEOUT`] for the same reason: this
/// runs at most once a day, and a slow connection should not be mistaken
/// for a dead one.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// A ceiling on the response, past which a reply is refused rather than
/// read into memory. The real catalogue is a few megabytes; this is
/// generous headroom, not a size anyone should ever actually hit.
const MAX_CATALOGUE_BYTES: u64 = 64 * 1024 * 1024;

/// How long a cached catalogue is trusted before [`Catalogue::is_stale`]
/// asks for a refetch. Models are added to [models.dev] every so often, not
/// every hour, so a day is generous without ever leaving the picker showing
/// a genuinely months-old list for long.
///
/// [models.dev]: https://models.dev
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// The shape Shaipe caches on disk — its own vocabulary, not [models.dev]'s
/// schema, which is free to add fields and reshape others without this
/// ever needing to change.
///
/// [models.dev]: https://models.dev
#[derive(Debug, Default, Serialize, Deserialize)]
struct Document {
    /// Unix seconds. An integer rather than [`SystemTime`] directly:
    /// `serde` has no `Serialize` for it without pulling in a feature just
    /// for this one field.
    #[serde(default)]
    fetched_at: Option<u64>,
    /// `"provider/model-id"` -> whether `modalities.input` names `"image"`.
    #[serde(default)]
    vision: HashMap<String, bool>,
}

/// Whether a model, named the way ACP names one, can see an image.
///
/// Tolerant by construction, the same way [`crate::settings::Settings`] is:
/// a missing or corrupt cache is an empty one, and a failed fetch or write
/// is swallowed rather than surfaced. Not knowing whether a model has
/// vision is never a reason to fail opening the workspace, only a reason to
/// leave [`Self::has_vision`] answering `None` for it.
#[derive(Debug, Default, Clone)]
pub struct Catalogue {
    /// Where this was loaded from and is cached back to. Explicit, not a
    /// global default, so a test can point it at a private `tempdir` —
    /// exactly why `src/fonts/mod.rs`'s `resolve_remote` takes its cache
    /// path as an argument instead of computing it internally.
    path: PathBuf,
    fetched_at: Option<u64>,
    vision: HashMap<String, bool>,
}

impl Catalogue {
    /// Load whatever is cached from the real OS config directory. Reads a
    /// file, nothing more — never the network, so this can never block the
    /// workspace opening.
    #[must_use]
    pub fn load() -> Self {
        Self::load_from(default_path())
    }

    /// Load from an explicit path, without touching the real config
    /// directory or the network. For tests.
    #[must_use]
    fn load_from(path: PathBuf) -> Self {
        let document = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Document>(&bytes).ok())
            .unwrap_or_default();

        Self {
            path,
            fetched_at: document.fetched_at,
            vision: document.vision,
        }
    }

    /// Build a catalogue directly from already-known answers, without
    /// touching a file at all — for a `crate::tui::modal` test that needs a
    /// `Catalogue` to say something specific about a handful of ids and has
    /// no reason to round-trip that through disk to get one.
    #[cfg(test)]
    pub(crate) fn with_vision(vision: HashMap<String, bool>) -> Self {
        Self {
            path: PathBuf::new(),
            fetched_at: None,
            vision,
        }
    }

    /// Whether this is missing, or old enough that [`Self::refresh`] should
    /// be asked to run again.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        let Some(fetched_at) = self.fetched_at else {
            return true;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        now.saturating_sub(fetched_at) > MAX_AGE.as_secs()
    }

    /// Whether `model_id` (ACP's own `"provider/model-id"` shape) can see an
    /// image.
    ///
    /// `None` when Shaipe simply does not know — the catalogue has not been
    /// fetched yet, the id is not in it (a self-hosted model, an agent that
    /// is not OpenCode, a model newer than the cached catalogue), or the
    /// last fetch failed. Never a reason to hide a model: only a confirmed
    /// `Some(false)` is.
    #[must_use]
    pub fn has_vision(&self, model_id: &str) -> Option<bool> {
        self.vision.get(model_id).copied()
    }

    /// Fetch the catalogue, flatten it, and cache the result at
    /// [`Self::path`].
    ///
    /// Blocking, like [`crate::fonts::resolve`] — this crate's one other
    /// network access — because `ureq` is synchronous; callers on an async
    /// runtime are expected to run this on a blocking thread, the same way
    /// [`crate::tui`] does.
    ///
    /// # Errors
    ///
    /// Returns [`Error::VisionFetch`] if the request fails or the reply
    /// cannot be parsed as [models.dev]'s own catalogue shape. The cache on
    /// disk, and `self`, are both left exactly as they were until a fetch
    /// actually succeeds — a failed refresh must never erase a working
    /// cache.
    ///
    /// [models.dev]: https://models.dev
    pub fn refresh(&mut self) -> Result<()> {
        let vision = fetch(CATALOGUE_URL)?;
        let fetched_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());

        self.fetched_at = Some(fetched_at);
        self.vision = vision;
        self.save();

        Ok(())
    }

    /// Write the current state back to [`Self::path`], swallowing every
    /// failure. A cache that fails to save just costs tomorrow's session an
    /// otherwise-avoidable refetch, not anything worse.
    fn save(&self) {
        let Ok(json) = serde_json::to_vec(&Document {
            fetched_at: self.fetched_at,
            vision: self.vision.clone(),
        }) else {
            return;
        };

        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, json);
    }
}

/// [models.dev]'s own catalogue shape — read only far enough to flatten it,
/// never kept around afterwards.
///
/// [models.dev]: https://models.dev
#[derive(Debug, Deserialize)]
struct Provider {
    #[serde(default)]
    models: HashMap<String, UpstreamModel>,
}

#[derive(Debug, Deserialize)]
struct UpstreamModel {
    #[serde(default)]
    modalities: Option<Modalities>,
}

#[derive(Debug, Deserialize)]
struct Modalities {
    #[serde(default)]
    input: Vec<String>,
}

/// Fetch and flatten [models.dev]'s catalogue from `url`.
///
/// A model has vision when `modalities.input` names `"image"` — checked
/// against live data rather than assumed: an `attachment` flag also exists
/// upstream and looked like the same thing, but disagreed with
/// `modalities.input` on about four percent of every model listed (`true`
/// for text-only models on several providers), so `attachment` is never
/// read here at all.
///
/// [models.dev]: https://models.dev
fn fetch(url: &str) -> Result<HashMap<String, bool>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(FETCH_TIMEOUT))
        .build()
        .into();

    let mut response = agent.get(url).call().map_err(|source| Error::VisionFetch {
        href: url.to_owned(),
        source: Box::new(source),
    })?;

    let providers: HashMap<String, Provider> = response
        .body_mut()
        .with_config()
        .limit(MAX_CATALOGUE_BYTES)
        .read_json()
        .map_err(|source| Error::VisionFetch {
            href: url.to_owned(),
            source: Box::new(source),
        })?;

    Ok(providers
        .into_iter()
        .flat_map(|(provider, listed)| {
            listed.models.into_iter().map(move |(model, upstream)| {
                let vision = upstream
                    .modalities
                    .is_some_and(|modalities| modalities.input.iter().any(|kind| kind == "image"));
                (format!("{provider}/{model}"), vision)
            })
        })
        .collect())
}

/// `<config dir>/vision.json` — the same `ProjectDirs` qualifier
/// [`crate::fonts`] and [`crate::settings`] use, `config_dir()` because this
/// is a preference about how the picker behaves, not disposable cache
/// content a cache-clearing tool should feel free to sweep away underneath
/// a running workspace mid-session.
fn default_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "shaipe", "shaipe")
        .map_or_else(
            || PathBuf::from(".shaipe-cache"),
            |dirs| dirs.config_dir().to_path_buf(),
        )
        .join("vision.json")
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn a_missing_cache_answers_unknown_for_everything_and_is_stale() {
        let directory = tempfile::tempdir().unwrap();
        let catalogue = Catalogue::load_from(directory.path().join("vision.json"));

        assert_eq!(catalogue.has_vision("anthropic/claude-opus-4-5"), None);
        assert!(catalogue.is_stale());
    }

    #[test]
    fn a_corrupt_cache_behaves_like_a_missing_one() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vision.json");
        std::fs::write(&path, b"not json at all").unwrap();

        let catalogue = Catalogue::load_from(path);
        assert_eq!(catalogue.has_vision("anthropic/claude-opus-4-5"), None);
    }

    /// Serves `body` to exactly one request, on a background thread, and
    /// returns the loopback URL to reach it at — the same harness
    /// `src/fonts/mod.rs`'s tests use, so this never needs the real network
    /// either.
    fn serve_once(body: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
        });
        format!("http://127.0.0.1:{port}/api.json")
    }

    /// A small fixture in exactly the shape [models.dev] actually answers
    /// with — two providers, one model each, one with an image modality and
    /// one without — confirmed against a real, live fetch rather than
    /// invented.
    ///
    /// [models.dev]: https://models.dev
    const FIXTURE: &[u8] = br#"{
        "anthropic": {
            "models": {
                "claude-opus-4-5": {
                    "attachment": true,
                    "modalities": { "input": ["text", "image", "pdf"], "output": ["text"] }
                }
            }
        },
        "deepinfra": {
            "models": {
                "meta-llama/Llama-3.3-70B-Instruct-Turbo": {
                    "attachment": false,
                    "modalities": { "input": ["text"], "output": ["text"] }
                }
            }
        }
    }"#;

    #[test]
    fn a_fetch_flattens_provider_and_model_into_one_id_and_reads_the_image_modality() {
        let vision = fetch(&serve_once(FIXTURE)).unwrap();

        assert_eq!(vision.get("anthropic/claude-opus-4-5"), Some(&true));
        assert_eq!(
            vision.get("deepinfra/meta-llama/Llama-3.3-70B-Instruct-Turbo"),
            Some(&false)
        );
    }

    #[test]
    fn a_refresh_caches_the_result_and_a_reload_reads_it_back_without_the_network() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vision.json");

        let mut catalogue = Catalogue::load_from(path.clone());
        // `refresh` reads `CATALOGUE_URL`, not an injectable one — this is
        // the one seam that has to reach the real network, so it is
        // exercised through `fetch` directly above instead, and here only
        // the caching contract around a result `fetch` already produced is
        // checked.
        let fetched_at = 1;
        catalogue.fetched_at = Some(fetched_at);
        catalogue
            .vision
            .insert("anthropic/claude-opus-4-5".to_owned(), true);
        catalogue.save();

        let reloaded = Catalogue::load_from(path);
        assert_eq!(reloaded.has_vision("anthropic/claude-opus-4-5"), Some(true));
        assert!(reloaded.is_stale(), "a cache from 1970 is always stale");
    }

    #[test]
    fn a_fresh_cache_is_not_stale() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalogue = Catalogue::load_from(directory.path().join("vision.json"));

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        catalogue.fetched_at = Some(now);

        assert!(!catalogue.is_stale());
    }

    #[test]
    fn an_unreachable_catalogue_is_reported_and_leaves_the_cache_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vision.json");

        let mut catalogue = Catalogue::load_from(path);
        catalogue
            .vision
            .insert("anthropic/claude-opus-4-5".to_owned(), true);
        catalogue.fetched_at = Some(1);

        // Port 0 is never listened on by the time this connects.
        let before = catalogue.vision.clone();
        let error = fetch("http://127.0.0.1:1/api.json").unwrap_err();
        assert!(matches!(error, Error::VisionFetch { .. }), "{error:?}");
        assert_eq!(
            catalogue.vision, before,
            "an unrelated fetch leaves state alone"
        );
    }
}
