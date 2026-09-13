//! Turning a declared font into bytes ready for `fontdb`.
//!
//! Two kinds of declaration, one job: [`resolve`] returns the bytes either
//! way. A local file is read exactly as it always was — resolved against the
//! project's own directory. A remote one is fetched once, its SHA-256
//! checked against what the project pinned, and cached content-addressed by
//! that hash — a warm cache reads nothing but that one file back, and never
//! the network again. This is the only place in the crate that makes an
//! outbound request; see ADR 015 for why, and why it is not
//! [`crate::render`], which still reads nothing but the project.
//!
//! A checksum is not optional for a remote font. Without one, a URL is just
//! system-font fallback with extra steps: the same untrusted "whatever is
//! there right now" that `--strict-fonts` exists to refuse. With one, the
//! cache key *is* the trust anchor — the same shape a lockfile gives a
//! package manager.

use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::project::Project;
use crate::project::font::{Font, FontSource};

/// How long a fetch may take before it is treated as unreachable.
///
/// Generous on purpose: this only ever runs once per machine per font, and a
/// slow-but-working connection should not be mistaken for a dead one.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Font files are small; refusing anything past a generous ceiling is
/// cheaper than explaining an out-of-memory crash to whoever declared a
/// `href` that pointed somewhere it should not have.
const MAX_FONT_BYTES: u64 = 32 * 1024 * 1024;

/// Where cached, checksum-verified font bytes live: `<cache
/// dir>/shaipe/fonts/<sha256>`. Never inside the project — a cache entry is
/// identified entirely by the hash the project already declared, so two
/// projects that pin the same font share one download.
///
/// # Errors
///
/// Returns [`Error::FontFetch`]-shaped context is not appropriate here since
/// this is a filesystem, not a network, lookup — callers get a plain
/// [`Error::Io`] if the OS refuses to say where its cache directory is,
/// which in practice does not happen on a machine capable of running
/// Shaipe at all.
fn cache_path(sha256: &str) -> PathBuf {
    directories::ProjectDirs::from("dev", "shaipe", "shaipe")
        // No project directories at all (an unusual, minimal environment) —
        // fall back to a directory relative to the process, so a fetch still
        // has somewhere to land rather than failing before it even tries.
        .map_or_else(
            || PathBuf::from(".shaipe-cache"),
            |dirs| dirs.cache_dir().to_path_buf(),
        )
        .join("fonts")
        .join(sha256)
}

/// Hex-encode a SHA-256 digest the same way every declared checksum is
/// written: 64 lowercase characters, no separators.
fn hex(bytes: impl AsRef<[u8]>) -> String {
    let mut out = String::with_capacity(64);
    for byte in Sha256::digest(bytes.as_ref()) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Turn a declared font into bytes.
///
/// # Errors
///
/// Returns [`Error::Font`] if a local file cannot be read,
/// [`Error::FontFetch`] if a remote one cannot be fetched, and
/// [`Error::FontChecksumMismatch`] if fetched or cached bytes do not match
/// the project's declared `sha256`.
pub fn resolve(project: &Project, font: &Font) -> Result<Vec<u8>> {
    match &font.source {
        FontSource::Local(src) => {
            let path = project.resolve(src);
            std::fs::read(&path).map_err(|source| Error::Font {
                family: font.family.clone(),
                path,
                source,
            })
        }
        FontSource::Remote { href, sha256 } => {
            resolve_remote(&cache_path(sha256), &font.family, href, sha256)
        }
    }
}

/// The remote half of [`resolve`], with the cache path an explicit
/// argument rather than computed inside — what lets a test point it at a
/// private `tempdir` instead of a real, shared, machine-wide cache
/// directory, without any global state (an env var, a `static`) that
/// parallel tests would race over.
fn resolve_remote(
    path: &std::path::Path,
    family: &str,
    href: &str,
    sha256: &str,
) -> Result<Vec<u8>> {
    if let Ok(cached) = std::fs::read(path) {
        let found = hex(&cached);
        if found == sha256 {
            // The steady state: read the cache, touch nothing else.
            return Ok(cached);
        }
        // A corrupted or truncated cache entry is not a checksum-mismatch
        // failure — that diagnostic is for a download that arrived wrong,
        // which the caller can only fix by declaring a different hash. A
        // bad *cache* is fixed by falling through and re-fetching, which is
        // exactly what happens next.
    }

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(FETCH_TIMEOUT))
        .build()
        .into();

    let mut response = agent.get(href).call().map_err(|source| Error::FontFetch {
        family: family.to_owned(),
        href: href.to_owned(),
        source: Box::new(source),
    })?;

    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_FONT_BYTES)
        .read_to_vec()
        .map_err(|source| Error::FontFetch {
            family: family.to_owned(),
            href: href.to_owned(),
            source: Box::new(source),
        })?;

    let found = hex(&bytes);
    if found != sha256 {
        return Err(Error::FontChecksumMismatch {
            family: family.to_owned(),
            href: href.to_owned(),
            expected: sha256.to_owned(),
            found,
        });
    }

    // Cached only after the checksum is confirmed — a mismatched download is
    // never written down for a later call to trust.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, &bytes);

    Ok(bytes)
}

/// Where a remote font's bytes would be cached, for reporting (`shaipe
/// inspect`) without resolving anything.
#[must_use]
pub fn remote_cache_path(sha256: &str) -> PathBuf {
    cache_path(sha256)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use pretty_assertions::assert_eq;

    use super::*;

    fn project() -> Project {
        crate::fixtures::project()
    }

    #[test]
    fn a_local_font_still_resolves_exactly_as_before() {
        let directory = tempfile::tempdir().unwrap();
        let font_path = directory.path().join("font.ttf");
        std::fs::write(&font_path, b"not a real font, just bytes").unwrap();

        let project = Project::from_source(
            directory.path().join("logo.svg"),
            crate::fixtures::PROJECT.to_owned(),
        )
        .unwrap();
        let font = Font::new("Inter", "font.ttf");

        assert_eq!(
            resolve(&project, &font).unwrap(),
            b"not a real font, just bytes"
        );
    }

    #[test]
    fn a_missing_local_font_reports_the_family_and_path() {
        let project = project();
        let font = Font::new("Inter", "missing.ttf");

        let error = resolve(&project, &font).unwrap_err();
        assert!(matches!(error, Error::Font { .. }), "{error:?}");
    }

    /// Serves `body` to exactly one request, on a background thread, and
    /// returns the loopback URL to reach it at. Not "the network" any more
    /// than the existing MCP bridge's own TCP loopback fallback is — nothing
    /// here leaves the machine, there is no DNS, and `mise run ci` never
    /// needs this thread to reach anywhere outside it.
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
        format!("http://127.0.0.1:{port}/font.ttf")
    }

    #[test]
    fn a_cache_miss_fetches_and_a_cache_hit_never_touches_the_server_again() {
        const BODY: &[u8] = b"pretend these bytes are a font file";
        let sha256 = hex(BODY);

        // A private cache path per test, passed explicitly — no env var, no
        // global state, so this is safe under `cargo test`'s default
        // parallel-by-default execution instead of merely hoping nothing
        // else touches `XDG_CACHE_HOME` at the same time.
        let cache_root = tempfile::tempdir().unwrap();
        let path = cache_root.path().join(&sha256);

        let href = serve_once(BODY);
        let bytes = resolve_remote(&path, "Test", &href, &sha256).unwrap();
        assert_eq!(bytes, BODY);
        assert!(path.exists(), "expected a cache entry at {path:?}");

        // The server above only ever answers one request — resolving again
        // must come from the cache or this would hang waiting for a second
        // connection nothing will accept.
        let bytes_again = resolve_remote(&path, "Test", &href, &sha256).unwrap();
        assert_eq!(bytes_again, BODY);
    }

    #[test]
    fn a_checksum_mismatch_is_rejected_and_never_cached() {
        const BODY: &[u8] = b"the actual bytes";
        let wrong_sha256 = hex(b"not the actual bytes");

        let cache_root = tempfile::tempdir().unwrap();
        let path = cache_root.path().join(&wrong_sha256);

        let href = serve_once(BODY);
        let error = resolve_remote(&path, "Test", &href, &wrong_sha256).unwrap_err();
        assert!(
            matches!(error, Error::FontChecksumMismatch { .. }),
            "{error:?}"
        );
        assert!(!path.exists(), "a mismatch must not be cached");
    }

    #[test]
    fn an_unreachable_server_reports_the_family_and_href() {
        let cache_root = tempfile::tempdir().unwrap();
        let path = cache_root.path().join("unreachable");
        // Port 0 is never listened on by the time this connects — nothing
        // accepts, so this fails fast rather than hanging.
        let error =
            resolve_remote(&path, "Test", "http://127.0.0.1:1/font.ttf", &hex(b"")).unwrap_err();
        assert!(matches!(error, Error::FontFetch { .. }), "{error:?}");
    }
}
