//! `shaipe doctor`.
//!
//! Prints what Shaipe can work out about the terminal it is attached to, and
//! why the preview backend is what it is.
//!
//! This exists because the failure it diagnoses is silent by construction: a
//! capability query that never gets an answer degrades to half-blocks, which
//! looks precisely like a terminal that genuinely cannot do better. The bug
//! that motivated it — a `StdoutLock` held across the workspace, which blocked
//! the query thread until it timed out — was invisible for exactly that reason.
//!
//! Runs without the alternate screen, so its own output is readable and can be
//! pasted into a bug report.

use shaipe::preview::{Backend, Preview, Scale};

use crate::cli::DoctorArgs;

/// Produce the report.
///
/// Returns a `String` rather than writing, and that is not a stylistic choice.
/// Detecting the terminal's capabilities means writing a query **from a spawned
/// thread** and reading the reply, so it must not happen while the stdout lock
/// is held — see `write_lines` in `main.rs`. Building the whole report first
/// keeps the query and the printing strictly apart.
///
/// The first version of this file got that wrong and ran under the lock, which
/// made `shaipe doctor` misdiagnose itself: the query was blocked until the
/// lock was released, so it was emitted *after* the report and every terminal
/// looked incapable.
#[must_use]
pub fn report(_args: &DoctorArgs, preview: Backend, scale: Option<Scale>) -> String {
    let mut report = String::new();
    environment(&mut report);
    detection(&mut report, preview, scale);
    report
}

/// What the environment says.
fn environment(out: &mut String) {
    use std::fmt::Write as _;

    let _ = writeln!(out, "terminal");
    for name in [
        "TERM",
        "TERM_PROGRAM",
        "TMUX",
        "KITTY_WINDOW_ID",
        "WEZTERM_EXECUTABLE",
        "KONSOLE_VERSION",
    ] {
        let value = std::env::var(name).unwrap_or_else(|_| "(unset)".to_owned());
        let _ = writeln!(out, "  {name:<20} {value}");
    }

    // Kitty graphics inside tmux need `allow-passthrough`, and the symptom of
    // it being off is an image that never appears with no error anywhere.
    if shaipe::preview::in_tmux() {
        let passthrough = std::process::Command::new("tmux")
            .args(["show", "-gv", "allow-passthrough"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map_or_else(
                || "(could not ask tmux)".to_owned(),
                |output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            );
        let _ = writeln!(out, "  {:<20} {passthrough}", "tmux passthrough");
    }
}

/// What the terminal itself says.
fn detection(out: &mut String, requested: Backend, scale: Option<Scale>) {
    use std::fmt::Write as _;

    let mut preview = Preview::detect(requested);
    if let Some(scale) = scale {
        preview.set_scale(scale);
    }
    let detected = preview.detection();

    let _ = writeln!(out, "\npreview");
    let _ = writeln!(out, "  {:<20} {}", "requested", requested);
    let _ = writeln!(
        out,
        "  {:<20} {}{}",
        "backend",
        detected.backend,
        if detected.forced { " (forced)" } else { "" }
    );
    let _ = writeln!(
        out,
        "  {:<20} {}x{} px",
        "cell size", detected.font_size.0, detected.font_size.1
    );
    let _ = writeln!(out, "  {:<20} {}", "tmux", detected.in_tmux);
    let _ = writeln!(
        out,
        "  {:<20} 1/{}{}",
        "transmit scale",
        preview.scale().factor(),
        if scale.is_some() { " (forced)" } else { "" }
    );

    match &detected.error {
        Some(reason) => {
            let _ = writeln!(out, "  {:<20} {reason}", "query failed");
            let _ = writeln!(
                out,
                "\nThe terminal did not answer the graphics capability query, so \
                 the preview\nfell back to half-blocks. If this terminal does \
                 support Kitty, Sixel or\niTerm2, that is a bug — please report \
                 this output."
            );
        }
        None if detected.backend == Backend::Blocks && !detected.forced => {
            // Deliberately hedged. `Picker::from_query_stdio` swallows
            // `NoCap`, `NoStdinResponse` and `NoFontSize` and returns a
            // half-blocks picker with no error, so from out here "answered
            // with nothing" and "never answered" are the same observation.
            // Claiming to know which would be a guess dressed as a diagnosis.
            let _ = writeln!(
                out,
                "\nNo graphics protocol was detected: the terminal either does \
                 not support one\nor did not answer. If it does support Kitty, \
                 Sixel or iTerm2, that is a bug —\nplease report this output."
            );
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_for(requested: Backend) -> String {
        super::report(&DoctorArgs {}, requested, None)
    }

    #[test]
    fn the_report_names_the_environment_variables_detection_depends_on() {
        let text = report_for(Backend::Blocks);
        for expected in ["TERM", "TERM_PROGRAM", "TMUX", "backend", "cell size"] {
            assert!(text.contains(expected), "{expected} missing from:\n{text}");
        }
    }

    #[test]
    fn a_forced_backend_is_reported_as_forced() {
        // Otherwise a report showing `blocks` is ambiguous between "detection
        // failed" and "the user asked for it", which is the whole question.
        assert!(report_for(Backend::Blocks).contains("(forced)"));
    }

    #[test]
    fn a_failed_query_is_reported_rather_than_silently_degraded() {
        // The test runner has no terminal to answer, so `auto` must fail here
        // — and must say so. This is the regression guard for the silent
        // fallback that made the original bug undiagnosable.
        let text = report_for(Backend::Auto);
        assert!(text.contains("query failed"), "{text}");
        assert!(text.contains("please report"), "{text}");
    }
}
