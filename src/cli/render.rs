//! `shaipe render`.
//!
//! The command the whole design is arranged around. It reads a project,
//! rasterises it, and writes files — with no network access, no model call and
//! no dependence on the machine it runs on. That is what makes it usable as a
//! CI step that regenerates a repository's assets and fails if they changed.

use std::io::Write;

use shaipe::project::{Background, Format, RenderSpec};
use shaipe::render::{FontPolicy, RenderOptions, Renderer};
use shaipe::{Project, Result};

use crate::cli::RenderArgs;

/// Run `shaipe render`.
///
/// # Errors
///
/// Returns whatever opening the project or rendering it returns. Nothing is
/// written before every specification has rendered successfully, so a failure
/// halfway through leaves no half-updated output directory behind.
pub fn run(args: &RenderArgs, out: &mut impl Write) -> Result<()> {
    let project = Project::open(&args.input)?;

    let options = RenderOptions {
        fonts: if args.strict_fonts {
            FontPolicy::Strict
        } else {
            FontPolicy::SystemFallback
        },
    };
    let renderer = Renderer::new(&project, options)?;

    let specs = specifications(&project, args)?;
    // Rendered in full before anything is written. A specification that fails
    // must not leave the output directory holding some of the new assets and
    // some of the old.
    let assets = specs
        .iter()
        .map(|spec| renderer.render(spec))
        .collect::<Result<Vec<_>>>()?;

    for asset in &assets {
        if args.dry_run {
            writeln!(
                out,
                "would write {}",
                args.output.join(asset.file_name()).display()
            )
            .map_err(|source| shaipe::Error::io(&args.output, source))?;
            continue;
        }

        let path = asset.write_to(&args.output)?;
        writeln!(
            out,
            "{} ({}x{})",
            path.display(),
            asset.spec.width,
            asset.spec.height
        )
        .map_err(|source| shaipe::Error::io(&path, source))?;
    }

    Ok(())
}

/// Decide what to render.
///
/// Three ways, in order of explicitness: an ad-hoc specification from the
/// flags, the named subset of the project's specifications, or all of them.
/// Clap has already ruled out combining the first with the others.
fn specifications(project: &Project, args: &RenderArgs) -> Result<Vec<RenderSpec>> {
    if let (Some(variant), Some(width)) = (&args.variant, args.width) {
        let background = args
            .background
            .as_deref()
            .map_or(Ok(Background::default()), str::parse)?;

        return Ok(vec![RenderSpec {
            // Named after the variant unless told otherwise, so
            // `--variant icon --width 64` writes `icon.png` rather than
            // demanding a name for a one-off.
            name: args.name.clone().unwrap_or_else(|| variant.clone()),
            variant: variant.clone(),
            width,
            height: args.height.unwrap_or(width),
            format: args.format.unwrap_or(Format::Png),
            background,
        }]);
    }

    if args.spec.is_empty() {
        return Ok(project.declared_renders()?.to_vec());
    }

    args.spec
        .iter()
        .map(|name| {
            project
                .metadata()
                .render_spec(name)
                .cloned()
                .ok_or_else(|| shaipe::Error::UnknownSpec {
                    spec: name.clone(),
                    known: project.metadata().spec_names(),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::cli::{Cli, Command};

    fn args(arguments: &[&str]) -> RenderArgs {
        let mut full = vec!["shaipe", "render"];
        full.extend_from_slice(arguments);
        match Cli::parse_from(full).command {
            Some(Command::Render(args)) => args,
            _ => panic!("expected a render command"),
        }
    }

    fn project() -> Project {
        Project::from_source("logo.svg", FIXTURE.to_owned()).unwrap()
    }

    const FIXTURE: &str = include_str!("../../tests/fixtures/logo.svg");

    #[test]
    fn with_no_flags_every_declared_specification_is_rendered() {
        let specs = specifications(&project(), &args(&[])).unwrap();
        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            ["favicon-32", "banner"]
        );
    }

    #[test]
    fn naming_specifications_renders_only_those_in_the_order_given() {
        let specs =
            specifications(&project(), &args(&["-s", "banner", "-s", "favicon-32"])).unwrap();
        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            ["banner", "favicon-32"]
        );
    }

    #[test]
    fn naming_a_specification_that_does_not_exist_says_which_ones_do() {
        let error = specifications(&project(), &args(&["-s", "og-image"])).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("og-image"), "{rendered}");
        assert!(matches!(error, shaipe::Error::UnknownSpec { .. }));
    }

    #[test]
    fn an_ad_hoc_render_defaults_to_a_square_named_after_its_variant() {
        let specs =
            specifications(&project(), &args(&["--variant", "icon", "--width", "64"])).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "icon");
        assert_eq!((specs[0].width, specs[0].height), (64, 64));
        assert_eq!(specs[0].format, Format::Png);
        assert_eq!(specs[0].background, Background::Transparent);
    }

    #[test]
    fn an_ad_hoc_render_takes_every_property_from_the_command_line() {
        let specs = specifications(
            &project(),
            &args(&[
                "--variant",
                "wordmark",
                "--width",
                "1280",
                "--height",
                "640",
                "--format",
                "svg",
                "--background",
                "#18181b",
                "--name",
                "social",
            ]),
        )
        .unwrap();

        assert_eq!(specs[0].name, "social");
        assert_eq!((specs[0].width, specs[0].height), (1280, 640));
        assert_eq!(specs[0].format, Format::Svg);
        assert_eq!(specs[0].background.to_string(), "#18181b");
    }

    #[test]
    fn an_unparsable_background_is_reported_rather_than_ignored() {
        let error = specifications(
            &project(),
            &args(&["--variant", "icon", "--width", "8", "--background", "puce"]),
        )
        .unwrap_err();
        assert!(matches!(error, shaipe::Error::InvalidColour { .. }));
    }

    #[test]
    fn a_dry_run_writes_no_files_but_names_all_of_them() {
        let directory = tempfile::tempdir().unwrap();
        let mut args = args(&["--dry-run"]);
        args.input = "tests/fixtures/logo.svg".into();
        args.output = directory.path().join("assets");

        let mut out = Vec::new();
        run(&args, &mut out).unwrap();

        let printed = String::from_utf8(out).unwrap();
        assert!(printed.contains("favicon-32.png"), "{printed}");
        assert!(printed.contains("banner.png"), "{printed}");
        assert!(!args.output.exists(), "a dry run must not create anything");
    }

    #[test]
    fn rendering_writes_one_file_per_specification() {
        let directory = tempfile::tempdir().unwrap();
        let mut args = args(&[]);
        args.input = "tests/fixtures/logo.svg".into();
        args.output = directory.path().join("assets");

        run(&args, &mut Vec::new()).unwrap();

        assert!(args.output.join("favicon-32.png").is_file());
        assert!(args.output.join("banner.png").is_file());
    }

    #[test]
    fn rendering_the_same_project_twice_writes_byte_identical_files() {
        // The property the assets CI check depends on: regenerating must
        // produce no diff unless the project actually changed.
        let directory = tempfile::tempdir().unwrap();
        let mut args = args(&[]);
        args.input = "tests/fixtures/logo.svg".into();

        args.output = directory.path().join("first");
        run(&args, &mut Vec::new()).unwrap();
        args.output = directory.path().join("second");
        run(&args, &mut Vec::new()).unwrap();

        for name in ["favicon-32.png", "banner.png"] {
            assert_eq!(
                std::fs::read(directory.path().join("first").join(name)).unwrap(),
                std::fs::read(directory.path().join("second").join(name)).unwrap(),
                "{name} differed between two runs"
            );
        }
    }
}
