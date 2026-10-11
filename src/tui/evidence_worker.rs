//! Reference evidence and comparison, off the terminal event loop.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use crate::compare::{self, Comparison};
use crate::preview::Image;
use crate::project::{Background, Format, Project, RenderSpec};
use crate::render::{RenderOptions, Renderer};

/// A complete answer for one source and variant. Keeping all four views together
/// means switching the visible view never repeats an expensive SSIM pass.
pub struct Evidence {
    pub reference: Image,
    pub current: Option<Image>,
    pub overlay: Option<Image>,
    pub difference: Option<Image>,
    pub comparison: Option<Comparison>,
    pub error: Option<String>,
    pub source: PathBuf,
    pub variant: Option<String>,
}

pub struct Answer {
    pub sequence: u64,
    pub result: Result<Evidence, String>,
}

struct Request {
    sequence: u64,
    project: Project,
    source: PathBuf,
    variant: Option<String>,
}

pub struct Worker {
    requests: Sender<Request>,
    answers: Receiver<Answer>,
}

impl Worker {
    pub fn new() -> Self {
        let (requests, incoming) = channel::<Request>();
        let (outgoing, answers) = channel();
        thread::spawn(move || {
            while let Ok(mut request) = incoming.recv() {
                // A burst of tabs or source selections should only compute its
                // final destination. Each request carries its own project, so a
                // coalesced reload cannot leave an older document in the worker.
                while let Ok(newer) = incoming.try_recv() {
                    request = newer;
                }
                let result = prepare(&request.project, &request.source, request.variant);
                if outgoing
                    .send(Answer {
                        sequence: request.sequence,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self { requests, answers }
    }

    pub fn request(
        &self,
        sequence: u64,
        project: &Project,
        source: PathBuf,
        variant: Option<String>,
    ) -> bool {
        self.requests
            .send(Request {
                sequence,
                project: project.clone(),
                source,
                variant,
            })
            .is_ok()
    }

    pub fn collect(&self) -> Option<Answer> {
        self.answers.try_recv().ok()
    }

    #[cfg(test)]
    pub fn wait(&self, timeout: std::time::Duration) -> Option<Answer> {
        self.answers.recv_timeout(timeout).ok()
    }
}

fn prepare(project: &Project, source: &Path, variant: Option<String>) -> Result<Evidence, String> {
    let path = project.resolve(source);
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("cannot read source {}: {error}", path.display()))?;
    // Use the analysis decoder before rendering: it rejects unsupported or
    // empty rasters and supplies the exact comparison canvas dimensions.
    let probe = crate::analysis::classify(&path, &bytes).map_err(|error| error.to_string())?;
    let decoded = image::load_from_memory(&bytes)
        .map_err(|error| format!("cannot decode {}: {error}", path.display()))?
        .to_rgba8();
    let reference = Image::from_rgba(decoded.width(), decoded.height(), decoded.into_raw());
    let mut evidence = Evidence {
        reference,
        current: None,
        overlay: None,
        difference: None,
        comparison: None,
        error: None,
        source: source.to_path_buf(),
        variant: variant.clone(),
    };
    if let Some(variant) = variant {
        let comparison = (|| {
            let spec = RenderSpec {
                name: variant.clone(),
                variant: variant.clone(),
                width: probe.width,
                height: probe.height,
                format: Format::Png,
                background: Background::Transparent,
            };
            let renderer = Renderer::new(project, RenderOptions::default())
                .map_err(|error| error.to_string())?;
            let asset = renderer.render(&spec).map_err(|error| error.to_string())?;
            let current = Image::from_png(&asset.bytes).map_err(|error| error.to_string())?;
            let label = PathBuf::from(format!("<{variant} render>"));
            let (report, images) = compare::compare(&path, &bytes, &label, &asset.bytes)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((current, report, images))
        })();
        match comparison {
            Ok((current, report, images)) => {
                evidence.current = Some(current);
                evidence.overlay =
                    Some(Image::from_png(&images.overlay).map_err(|error| error.to_string())?);
                evidence.difference =
                    Some(Image::from_png(&images.difference).map_err(|error| error.to_string())?);
                evidence.comparison = Some(report);
            }
            Err(error) => evidence.error = Some(error),
        }
    }
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn project_with_source(dir: &std::path::Path) -> Project {
        let path = dir.join("logo.svg");
        let mut project = Project::from_source(path, fixtures::PROJECT.to_owned()).unwrap();
        project
            .metadata_mut()
            .references
            .push(crate::project::Reference::new(
                "reference.png",
                crate::project::ReferenceKind::Source,
            ));
        let image = Image::from_rgba(32, 24, vec![255; 32 * 24 * 4]);
        std::fs::write(dir.join("reference.png"), image.to_png().unwrap()).unwrap();
        project
    }

    #[test]
    fn comparison_uses_the_reference_canvas_and_provides_every_view() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = project_with_source(dir.path());
        // A declared output may be much bigger than the comparison target.
        project.metadata_mut().renders[0].width = 512;
        let worker = Worker::new();
        assert!(worker.request(1, &project, "reference.png".into(), Some("icon".into())));
        let answer = worker.wait(std::time::Duration::from_secs(10)).unwrap();
        let evidence = answer.result.unwrap();
        assert_eq!(answer.sequence, 1);
        assert_eq!(
            (evidence.reference.width(), evidence.reference.height()),
            (32, 24)
        );
        assert_eq!(
            (
                evidence.current.as_ref().unwrap().width(),
                evidence.current.as_ref().unwrap().height()
            ),
            (32, 24)
        );
        assert!(evidence.overlay.is_some() && evidence.difference.is_some());
        assert_eq!(evidence.comparison.as_ref().unwrap().canvas.width, 32);
    }

    #[test]
    fn a_missing_variant_does_not_hide_the_reference() {
        let dir = tempfile::tempdir().unwrap();
        let project = project_with_source(dir.path());
        let evidence =
            prepare(&project, Path::new("reference.png"), Some("absent".into())).unwrap();
        assert_eq!(evidence.reference.width(), 32);
        assert!(evidence.comparison.is_none());
        assert!(evidence.error.is_some());
    }

    #[test]
    fn invalid_source_is_reported_without_killing_the_worker() {
        let dir = tempfile::tempdir().unwrap();
        let project = project_with_source(dir.path());
        let worker = Worker::new();
        worker.request(1, &project, "missing.png".into(), Some("icon".into()));
        assert!(
            worker
                .wait(std::time::Duration::from_secs(10))
                .unwrap()
                .result
                .is_err()
        );
        worker.request(2, &project, "reference.png".into(), Some("icon".into()));
        assert!(
            worker
                .wait(std::time::Duration::from_secs(10))
                .unwrap()
                .result
                .is_ok()
        );
    }
}
