//! `shaipe inspect`.
//!
//! Answers "what is in this file?" in two registers. The text form is for a
//! person; `--format json` is the stable surface an agent reads, and is the
//! reason [`shaipe::inspect::Report`] lives in the library rather than here.

use std::io::Write;

use shaipe::inspect::{FontSourceReport, Report};
use shaipe::{Error, Project, Result};

use crate::cli::{InspectArgs, ReportFormat};

/// Run `shaipe inspect`.
///
/// # Errors
///
/// Returns whatever opening the project returns, or [`Error::Io`] if the
/// report cannot be written out.
pub fn run(args: &InspectArgs, out: &mut dyn Write) -> Result<()> {
    let project = Project::open(&args.input)?;
    let report = Report::of(&project);

    let rendered = match args.format {
        // Pretty-printed because this output is read by people in a terminal
        // and committed to fixtures; a machine does not care either way.
        ReportFormat::Json => {
            serde_json::to_string_pretty(&report).expect("a report is always serialisable") + "\n"
        }
        ReportFormat::Text => text(&report),
    };

    out.write_all(rendered.as_bytes())
        .map_err(|source| Error::io(&args.input, source))
}

/// Render a report for a person.
fn text(report: &Report) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    let _ = writeln!(out, "{}", report.path.display());
    if let Some(prompt) = &report.prompt {
        let _ = writeln!(out, "\n  {prompt}");
    }

    if let Some(generation) = &report.generation {
        let _ = write!(out, "\ngenerated");
        for (label, value) in [
            ("by", &generation.agent),
            ("with", &generation.model),
            ("at", &generation.at),
        ] {
            if let Some(value) = value {
                let _ = write!(out, " {label} {value}");
            }
        }
        out.push('\n');
    }

    let _ = write!(out, "\nvariants");
    if report.variants.is_empty() {
        out.push_str("  (none)\n");
    } else {
        out.push('\n');
        for variant in &report.variants {
            let primary = if report.primary.as_deref() == Some(variant.name.as_str()) {
                "  (primary)"
            } else {
                ""
            };
            let _ = writeln!(out, "  {:<16} #{}{primary}", variant.name, variant.element);
        }
    }

    let _ = write!(out, "\npalette");
    if report.palette.is_empty() {
        out.push_str("   (none)\n");
    } else {
        out.push('\n');
        for colour in &report.palette {
            let role = colour.role.as_deref().unwrap_or_default();
            let _ = writeln!(out, "  {:<16} {:<10} {role}", colour.name, colour.value);
        }
    }

    if !report.fonts.is_empty() {
        out.push_str("\nfonts\n");
        for font in &report.fonts {
            // A missing local file, or a not-yet-cached remote one, is the
            // single most likely cause of a render that differs between a
            // laptop and CI, so it is called out here rather than left to be
            // discovered at render time.
            match &font.source {
                FontSourceReport::Local { src, present, .. } => {
                    let status = if *present { "" } else { "  MISSING" };
                    let _ = writeln!(out, "  {:<16} {}{status}", font.family, src.display());
                }
                FontSourceReport::Remote { href, cached, .. } => {
                    let status = if *cached {
                        "  (cached)"
                    } else {
                        "  NOT CACHED"
                    };
                    let _ = writeln!(out, "  {:<16} {href}{status}", font.family);
                }
            }
        }
    }

    if !report.references.is_empty() {
        out.push_str("\nreferences\n");
        for reference in &report.references {
            let status = if reference.present { "" } else { "  MISSING" };
            let _ = writeln!(
                out,
                "  {:<16} {}{status}",
                reference.kind,
                reference.src.display()
            );
        }
    }

    let _ = write!(out, "\nrenders");
    if report.renders.is_empty() {
        out.push_str("   (none)\n");
    } else {
        out.push('\n');
        for spec in &report.renders {
            let _ = writeln!(
                out,
                "  {:<16} {:<12} {}x{} on {}",
                spec.name, spec.variant, spec.width, spec.height, spec.background
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        let project = Project::open("tests/fixtures/logo.svg").unwrap();
        Report::of(&project)
    }

    #[test]
    fn the_text_report_names_every_variant_palette_entry_and_render() {
        let rendered = text(&report());

        for expected in [
            "icon",
            "wordmark",
            "mark-wide",
            "accent",
            "#f05032",
            "favicon-32",
            "banner",
        ] {
            assert!(
                rendered.contains(expected),
                "{expected} missing from:\n{rendered}"
            );
        }
    }

    #[test]
    fn the_text_report_marks_which_variant_the_document_root_draws() {
        // Which variant a browser shows is not otherwise discoverable without
        // reading the XML.
        assert!(text(&report()).contains("(primary)"));
    }

    #[test]
    fn the_json_report_is_valid_json_and_carries_the_schema_version() {
        let rendered = serde_json::to_string(&report()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["variants"][0]["name"], "icon");
    }
}
