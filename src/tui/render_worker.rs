//! Rendering, off the drawing thread.
//!
//! Rendering used to happen inline, immediately before the frame was drawn.
//! That made the `rendering…` placeholder unreachable — the frame could only
//! ever be drawn *after* the render finished — and it froze the workspace for
//! the duration of every rasterise and every image transmission. In a terminal
//! speaking the Kitty protocol that is over a megabyte of escape sequences per
//! change, which is where "it takes ten seconds" came from.
//!
//! So a worker owns the [`Renderer`] and the project, and the drawing thread
//! only ever asks and collects. Two things fall out of that which are worth
//! more than the threading itself:
//!
//! - the font database is built once, not once per preview;
//! - requests coalesce. Holding an arrow key queues a request per repeat, and
//!   all but the last are dropped before any work starts.

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread;
use std::time::{Duration, Instant};

use crate::preview::Image;
use crate::project::{Project, RenderSpec};
use crate::render::{RenderOptions, Renderer};

/// What the drawing thread asks for.
enum Request {
    /// Render this specification. `seq` identifies the answer.
    Render { seq: u64, spec: Box<RenderSpec> },
    /// The project changed; rebuild the renderer around it.
    Reload(Box<Project>),
}

/// What comes back.
pub struct Rendered {
    /// Which request this answers.
    pub seq: u64,
    /// The pixels, or why there are none.
    pub image: Result<Image, String>,
    /// How long the render took, for the status line under `-v`.
    pub elapsed: Duration,
}

/// A thread that renders previews.
///
/// Dropping it closes the request channel, which ends the thread. The join
/// handle is deliberately not kept: a worker blocked on a slow rasterise must
/// not be able to hold up quitting the application.
pub struct Worker {
    requests: Sender<Request>,
    results: Receiver<Rendered>,
}

impl Worker {
    /// Start a worker for a project.
    #[must_use]
    pub fn new(project: &Project) -> Self {
        let (request_tx, request_rx) = channel::<Request>();
        let (result_tx, result_rx) = channel::<Rendered>();
        let mut project = project.clone();

        thread::spawn(move || {
            while let Ok(request) = request_rx.recv() {
                // Everything queued behind this one is already stale, so it is
                // dropped before any of it is rendered. This is what makes
                // holding an arrow key cost one render rather than thirty.
                //
                // Only *renders* are stale, though. A `Reload` carries the
                // project every later render will be drawn from, so it is
                // applied as it goes past rather than overwritten by whatever
                // followed it. Dropping one left the worker rendering the old
                // project for the rest of the session — which is what an
                // agent's edit hit whenever it landed while a rasterise was
                // already running.
                let mut latest = None;
                let mut next = Some(request);
                loop {
                    match next.take() {
                        Some(Request::Reload(updated)) => project = *updated,
                        // Only the last one survives; that is the coalescing.
                        Some(render) => latest = Some(render),
                        None => {}
                    }
                    match request_rx.try_recv() {
                        Ok(request) => next = Some(request),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }

                let Some(Request::Render { seq, spec }) = latest else {
                    continue;
                };

                let started = Instant::now();
                let image = Renderer::new(&project, RenderOptions::default())
                    .and_then(|renderer| renderer.pixels(&spec))
                    .map(|pixmap| Image::from_pixmap(&pixmap))
                    .map_err(|error| error.to_string());

                if result_tx
                    .send(Rendered {
                        seq,
                        image,
                        elapsed: started.elapsed(),
                    })
                    .is_err()
                {
                    return;
                }
            }
        });

        Self {
            requests: request_tx,
            results: result_rx,
        }
    }

    /// Ask for a render.
    ///
    /// Returns `false` if the worker has gone away, which the caller reports
    /// rather than panicking on: a workspace that cannot render is still worth
    /// having open to read the project.
    pub fn request(&self, seq: u64, spec: RenderSpec) -> bool {
        self.requests
            .send(Request::Render {
                seq,
                spec: Box::new(spec),
            })
            .is_ok()
    }

    /// Tell the worker the project changed.
    pub fn reload(&self, project: &Project) -> bool {
        self.requests
            .send(Request::Reload(Box::new(project.clone())))
            .is_ok()
    }

    /// Collect a finished render, if one is ready. Never blocks.
    pub fn collect(&self) -> Option<Rendered> {
        self.results.try_recv().ok()
    }

    /// Wait for a finished render.
    ///
    /// Used to draw the first preview, and by tests, which would otherwise
    /// have to sleep and hope.
    #[must_use]
    pub fn wait(&self, timeout: Duration) -> Option<Rendered> {
        self.results.recv_timeout(timeout).ok()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures;

    const PATIENCE: Duration = Duration::from_secs(10);

    #[test]
    fn a_worker_renders_what_it_is_asked_for() {
        let worker = Worker::new(&fixtures::project());
        assert!(worker.request(1, RenderSpec::square("p", "icon", 32)));

        let rendered = worker.wait(PATIENCE).expect("a render");
        assert_eq!(rendered.seq, 1);
        let image = rendered.image.expect("pixels");
        assert_eq!((image.width(), image.height()), (32, 32));
    }

    #[test]
    fn a_failed_render_comes_back_as_a_message_rather_than_a_panic() {
        // The worker thread must survive a project that will not render, or
        // one bad edit would leave the workspace permanently blank.
        let worker = Worker::new(&fixtures::project());
        worker.request(1, RenderSpec::square("p", "nonexistent", 32));

        let rendered = worker.wait(PATIENCE).expect("an answer");
        assert!(rendered.image.is_err());

        // And it still works afterwards.
        worker.request(2, RenderSpec::square("p", "icon", 16));
        assert!(worker.wait(PATIENCE).expect("a render").image.is_ok());
    }

    #[test]
    fn a_reload_queued_behind_a_render_is_not_discarded() {
        // Coalescing used to overwrite whatever it was holding, so a `Reload`
        // sitting behind a `Render` was thrown away and the worker rendered
        // the *old* project for the rest of the session. That is what an
        // agent's edit hit whenever it landed while a rasterise was already
        // running: `write_svg` succeeded, the preview never changed.
        //
        // Both are sent before the worker is given a chance to run, so they
        // are certainly drained in one pass — which is the case that used to
        // lose one.
        let worker = Worker::new(&fixtures::project());

        let mut edited = fixtures::project();
        edited.metadata_mut().palette = crate::project::Palette::default();

        worker.request(1, RenderSpec::square("p", "icon", 16));
        worker.reload(&edited);
        worker.request(2, RenderSpec::square("p", "icon", 16));

        // Drain whatever the pass produced. A short wait: this is only
        // clearing the decks, and PATIENCE here would be paid in full on the
        // last iteration for nothing.
        while worker.wait(Duration::from_millis(400)).is_some() {}

        // The proof: ask for a variant that only exists in the *original*
        // project. If the reload was dropped the worker still has it and the
        // render succeeds; if the reload landed, it cannot.
        let mut without = fixtures::project();
        without.metadata_mut().variants.clear();
        worker.reload(&without);
        worker.request(3, RenderSpec::square("p", "icon", 16));

        let answer = worker.wait(PATIENCE).expect("a render");
        assert!(
            answer.image.is_err(),
            "the worker is still rendering a project it was told to replace"
        );
    }

    #[test]
    fn a_reload_is_applied_even_when_a_render_follows_it_immediately() {
        // The same thing from the other side, and the ordering that matters:
        // whatever the queue order, the render must be drawn from the newest
        // project.
        let worker = Worker::new(&fixtures::project());

        let mut without = fixtures::project();
        without.metadata_mut().variants.clear();

        worker.reload(&without);
        worker.request(1, RenderSpec::square("p", "icon", 16));

        let answer = worker.wait(PATIENCE).expect("a render");
        assert!(
            answer.image.is_err(),
            "the render was drawn from the project the reload replaced"
        );
    }

    #[test]
    fn queued_requests_coalesce_to_the_last_one() {
        // The whole point: arrowing through ten entries must not rasterise ten
        // images, nine of which nobody ever sees.
        let worker = Worker::new(&fixtures::project());
        for seq in 1..=8 {
            worker.request(
                seq,
                RenderSpec::square("p", "icon", 16 * u32::try_from(seq).unwrap()),
            );
        }

        let mut answers = Vec::new();
        while let Some(rendered) = worker.wait(Duration::from_millis(400)) {
            answers.push(rendered.seq);
        }

        assert!(!answers.is_empty(), "at least one render should happen");
        assert_eq!(
            *answers.last().unwrap(),
            8,
            "the most recent request must be the one that is answered"
        );
        assert!(
            answers.len() < 8,
            "requests should coalesce, but all {} were rendered",
            answers.len()
        );
    }

    #[test]
    fn collecting_when_nothing_is_ready_does_not_block() {
        let worker = Worker::new(&fixtures::project());
        assert!(worker.collect().is_none());
    }
}
