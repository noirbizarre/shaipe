//! Bounded, session-only snapshots of the artwork and its visual evidence.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, channel, sync_channel};
use std::thread;

use crate::compare::Comparison;
use crate::preview::Image;
use crate::project::{Format, Project, RenderSpec};
use crate::render::{RenderOptions, Renderer};
use crate::workflow::{Phase, WorkflowKind};

use super::evidence_worker::{self, Evidence};

const LIMIT: usize = 12;
const IMAGE_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Previous,
    Next,
    Live,
    Approve,
    Reject,
    Restore,
}

#[derive(Default)]
pub struct Context {
    pub mode: super::app::Mode,
    pub variant: Option<String>,
    pub spec: Option<RenderSpec>,
    pub kind: Option<WorkflowKind>,
    pub phase: Option<Phase>,
    pub source: Option<PathBuf>,
}

pub struct Iteration {
    pub id: u64,
    pub project: Project,
    pub svg: String,
    pub variant: Option<String>,
    pub mode: super::app::Mode,
    pub spec: Option<RenderSpec>,
    pub kind: Option<WorkflowKind>,
    pub phase: Option<Phase>,
    pub origin: String,
    pub decision: Decision,
    pub preview: Option<Image>,
    pub evidence: Option<Evidence>,
    pub error: Option<String>,
    pub pending: bool,
}

impl Iteration {
    pub fn image(&self, view: super::app::EvidenceView) -> Option<&Image> {
        match view {
            super::app::EvidenceView::Current => self.preview.as_ref(),
            super::app::EvidenceView::Reference => self.evidence.as_ref().map(|e| &e.reference),
            super::app::EvidenceView::Overlay => self.evidence.as_ref()?.overlay.as_ref(),
            super::app::EvidenceView::Difference => self.evidence.as_ref()?.difference.as_ref(),
        }
    }

    pub fn comparison(&self) -> Option<&Comparison> {
        self.evidence.as_ref()?.comparison.as_ref()
    }

    fn image_bytes(&self) -> usize {
        let mut size = self
            .preview
            .as_ref()
            .map_or(0, |image| image.pixels().len());
        if let Some(e) = &self.evidence {
            size += e.reference.pixels().len();
            size += e.current.as_ref().map_or(0, |i| i.pixels().len());
            size += e.overlay.as_ref().map_or(0, |i| i.pixels().len());
            size += e.difference.as_ref().map_or(0, |i| i.pixels().len());
        }
        size
    }
}

struct Request {
    id: u64,
    project: Project,
    spec: Option<RenderSpec>,
    source: Option<(PathBuf, Result<Vec<u8>, String>)>,
    variant: Option<String>,
}

struct Answer {
    id: u64,
    preview: Result<Option<Image>, String>,
    evidence: Option<Result<Evidence, String>>,
}

pub struct History {
    entries: VecDeque<Iteration>,
    selected: Option<u64>,
    next_id: u64,
    requests: SyncSender<Request>,
    answers: Receiver<Answer>,
}

impl History {
    pub fn new() -> Self {
        // The workspace cannot afford an unlimited queue of image-sized
        // projects when an agent submits many edits faster than comparisons.
        let (requests, incoming) = sync_channel::<Request>(LIMIT);
        let (outgoing, answers) = channel::<Answer>();
        thread::spawn(move || {
            while let Ok(request) = incoming.recv() {
                let preview = request.spec.map_or(Ok(None), |mut spec| {
                    // Historical previews have a fixed bound independent of terminal size.
                    spec.width = spec.width.clamp(1, 512);
                    spec.height = spec.height.clamp(1, 512);
                    spec.format = Format::Png;
                    Renderer::new(&request.project, RenderOptions::default())
                        .and_then(|renderer| renderer.pixels(&spec))
                        .map(|pixels| Some(Image::from_pixmap(&pixels)))
                        .map_err(|error| error.to_string())
                });
                let evidence = request.source.map(|(path, bytes)| {
                    bytes.and_then(|bytes| {
                        // Compare at native resolution, but refuse a raster
                        // whose four decoded views would exceed the bounded
                        // history image budget before allocating them.
                        let dimensions = image::ImageReader::new(std::io::Cursor::new(&bytes))
                            .with_guessed_format()
                            .map_err(|error| error.to_string())?
                            .into_dimensions()
                            .map_err(|error| error.to_string())?;
                        if dimensions.0 > 1_600 || dimensions.1 > 1_600 {
                            return Err(format!(
                                "reference {} exceeds the 1600px history canvas limit",
                                path.display()
                            ));
                        }
                        evidence_worker::prepare_bytes(
                            &request.project,
                            &path,
                            &bytes,
                            request.variant,
                        )
                    })
                });
                if outgoing
                    .send(Answer {
                        id: request.id,
                        preview,
                        evidence,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            entries: VecDeque::new(),
            selected: None,
            next_id: 0,
            requests,
            answers,
        }
    }

    pub fn entries(&self) -> &VecDeque<Iteration> {
        &self.entries
    }

    pub fn selected(&self) -> Option<&Iteration> {
        let id = self.selected?;
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn is_browsing(&self) -> bool {
        self.selected.is_some()
    }

    pub fn record(&mut self, project: &Project, context: Context, origin: String) -> bool {
        let Ok(svg) = project.to_svg() else {
            return false;
        };
        if self.entries.back().is_some_and(|last| last.svg == svg) {
            return false;
        }
        self.next_id += 1;
        let id = self.next_id;
        // Freeze the reference bytes when the candidate is created, not when a
        // queued worker finally gets to it after an external file edit.
        let source = context.source.map(|path| {
            let resolved = project.resolve(&path);
            let bytes = std::fs::read(&resolved)
                .map_err(|error| format!("cannot read source {}: {error}", resolved.display()));
            let bytes = bytes.and_then(|bytes| {
                if bytes.len() > 16 * 1024 * 1024 {
                    Err(format!(
                        "reference {} exceeds the 16 MiB history limit",
                        resolved.display()
                    ))
                } else {
                    Ok(bytes)
                }
            });
            (path, bytes)
        });
        self.entries.push_back(Iteration {
            id,
            project: project.clone(),
            svg,
            variant: context.variant.clone(),
            mode: context.mode,
            spec: context.spec.clone(),
            kind: context.kind,
            phase: context.phase,
            // The transcript keeps the full conversation. The strip only
            // needs a readable, bounded attribution, never a multiline turn.
            origin: origin
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(512)
                .collect(),
            decision: Decision::Pending,
            preview: None,
            evidence: None,
            error: None,
            pending: true,
        });
        if self
            .requests
            .try_send(Request {
                id,
                project: project.clone(),
                spec: context.spec,
                source,
                variant: context.variant,
            })
            .is_err()
            && let Some(last) = self.entries.back_mut()
        {
            last.pending = false;
            last.error =
                Some("history evidence is unavailable: the worker is busy or stopped".to_owned());
        }
        self.trim();
        true
    }

    pub fn collect(&mut self) -> bool {
        let mut changed = false;
        while let Ok(answer) = self.answers.try_recv() {
            changed |= self.apply(answer);
        }
        if changed {
            self.trim();
        }
        changed
    }

    fn apply(&mut self, answer: Answer) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == answer.id) else {
            return false;
        };
        entry.pending = false;
        match answer.preview {
            Ok(preview) => entry.preview = preview,
            Err(error) => entry.error = Some(error),
        }
        if let Some(evidence) = answer.evidence {
            match evidence {
                Ok(evidence) => entry.evidence = Some(evidence),
                Err(error) => entry.error = Some(error),
            }
        }
        true
    }

    #[cfg(test)]
    fn wait(&mut self) {
        let answer = self
            .answers
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        self.apply(answer);
    }

    fn trim(&mut self) {
        while self.entries.len() > LIMIT
            || self
                .entries
                .iter()
                .map(Iteration::image_bytes)
                .sum::<usize>()
                > IMAGE_LIMIT
        {
            // Never evict the iteration being inspected; discard the oldest
            // other candidate, even when a late worker answer grows the cache.
            let latest = self.entries.back().map(|entry| entry.id);
            let index = self
                .entries
                .iter()
                .position(|e| Some(e.id) != self.selected && Some(e.id) != latest);
            if let Some(index) = index.filter(|_| self.entries.len() > 1) {
                self.entries.remove(index);
            } else {
                // Two deliberately protected candidates may exceed the image
                // budget. Keep their SVG and review decisions, but release the
                // live candidate's derived images before risking its loss.
                if self
                    .entries
                    .iter()
                    .map(Iteration::image_bytes)
                    .sum::<usize>()
                    > IMAGE_LIMIT
                    && let Some(last) = self.entries.back_mut()
                {
                    last.preview = None;
                    last.evidence = None;
                    last.error = Some("history images exceeded the 64 MiB session limit".into());
                }
                break;
            }
        }
    }

    pub fn step(&mut self, direction: isize) {
        let Some(current) = self
            .selected
            .and_then(|id| self.entries.iter().position(|e| e.id == id))
            .or_else(|| self.entries.len().checked_sub(1))
        else {
            return;
        };
        let next =
            (current as isize + direction).clamp(0, self.entries.len() as isize - 1) as usize;
        self.selected = Some(self.entries[next].id);
    }

    pub fn live(&mut self) {
        self.selected = None;
    }

    pub fn decide(&mut self, decision: Decision) {
        let id = self.selected.or_else(|| self.entries.back().map(|e| e.id));
        if let Some(entry) = self.entries.iter_mut().find(|e| Some(e.id) == id) {
            entry.decision = decision;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    #[test]
    fn snapshots_keep_unsaved_svg_and_review_decisions_out_of_the_project() {
        let mut history = History::new();
        let mut project = fixtures::project();
        assert!(history.record(&project, Context::default(), "opened".into()));
        project.metadata_mut().description = Some("new intent".into());
        assert!(history.record(&project, Context::default(), "edited".into()));
        assert!(!history.record(&project, Context::default(), "duplicate".into()));
        history.step(-1);
        assert_eq!(
            history.selected().unwrap().project.metadata().description,
            fixtures::project().metadata().description
        );
        history.decide(Decision::Rejected);
        assert_eq!(history.selected().unwrap().decision, Decision::Rejected);
        assert!(!project.to_svg().unwrap().contains("rejected"));
    }

    #[test]
    fn bounded_history_retains_the_inspected_candidate_and_ignores_evicted_answers() {
        let mut history = History::new();
        let mut project = fixtures::project();
        history.record(&project, Context::default(), "opened".into());
        history.step(-1);
        let selected = history.selected().unwrap().id;
        for index in 0..LIMIT + 2 {
            project.metadata_mut().description = Some(format!("iteration {index}"));
            history.record(&project, Context::default(), format!("edit {index}"));
        }
        history.collect();
        assert_eq!(history.entries().len(), LIMIT);
        assert_eq!(history.selected().unwrap().id, selected);
    }

    #[test]
    fn historical_evidence_uses_the_reference_bytes_from_capture_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.png");
        let white = Image::from_rgba(32, 32, vec![255; 32 * 32 * 4]);
        std::fs::write(&path, white.to_png().unwrap()).unwrap();
        let mut project =
            Project::from_source(dir.path().join("logo.svg"), fixtures::PROJECT.to_owned())
                .unwrap();
        project
            .metadata_mut()
            .references
            .push(crate::project::Reference::new(
                "source.png",
                crate::project::ReferenceKind::Source,
            ));
        let mut history = History::new();
        history.record(
            &project,
            Context {
                variant: Some("icon".into()),
                spec: Some(RenderSpec::square("icon", "icon", 64)),
                source: Some("source.png".into()),
                ..Context::default()
            },
            "candidate".into(),
        );
        std::fs::write(
            &path,
            Image::from_rgba(32, 32, vec![0; 32 * 32 * 4])
                .to_png()
                .unwrap(),
        )
        .unwrap();
        history.wait();
        history.step(-1);
        let entry = history.selected().unwrap();
        assert!(entry.preview.is_some());
        assert_eq!(
            entry.evidence.as_ref().unwrap().reference.pixel(0, 0),
            [255; 4]
        );
        assert!(entry.comparison().is_some());
        assert!(
            entry
                .image(super::super::app::EvidenceView::Overlay)
                .is_some()
        );
        assert!(
            entry
                .image(super::super::app::EvidenceView::Difference)
                .is_some()
        );
    }
}
