//! The card preview the browser companion shows, rendered by the one
//! `lvgl_sim::Simulator` this process owns.
//!
//! The dedicated thread is required by the simulator: `Simulator` is not `Sync`
//! and must never migrate between threads, so exactly one is claimed for the
//! process lifetime and driven from a single thread.
//!
//! Requests arrive on an `mpsc` channel with latest-wins coalescing: if more than
//! one render is queued when the thread wakes, only the newest renders and every
//! superseded request is told so explicitly rather than left waiting for an answer
//! about a card the operator has already navigated away from.
//!
//! Callers are concurrent HTTP handlers. `render` blocks only its calling thread,
//! and every axum handler that uses it does so inside `spawn_blocking`.

use std::sync::mpsc;

pub(crate) struct PreviewHandle {
    sender: mpsc::Sender<PreviewJob>,
}

struct PreviewJob {
    request: lvgl_sim::scene::SceneRenderRequest,
    reply: mpsc::Sender<Result<Vec<u8>, String>>,
}

pub(crate) fn spawn() -> PreviewHandle {
    let (sender, receiver) = mpsc::channel::<PreviewJob>();
    std::thread::Builder::new()
        .name("preview-sim".into())
        .spawn(move || {
            let mut simulator = match lvgl_sim::Simulator::new() {
                Ok(simulator) => simulator,
                Err(error) => {
                    // Drain forever with the error: "preview unavailable", never a
                    // stale or invented frame.
                    while let Ok(job) = receiver.recv() {
                        let _ = job
                            .reply
                            .send(Err(format!("simulator init failed: {error:?}")));
                    }
                    return;
                }
            };
            while let Ok(mut job) = receiver.recv() {
                // Coalesce: only the newest queued request renders.
                while let Ok(newer) = receiver.try_recv() {
                    let _ = job.reply.send(Err(SUPERSEDED.to_owned()));
                    job = newer;
                }
                let result = simulator
                    .render_scene_png(&job.request)
                    .map_err(|error| format!("render failed: {error:?}"));
                let _ = job.reply.send(result);
            }
        })
        .expect("spawn preview thread");
    PreviewHandle { sender }
}

/// The reply a request gets when a newer one for the same simulator arrived while
/// it was still queued. Named because the HTTP layer reports it as a retryable
/// condition rather than as a render failure.
pub(crate) const SUPERSEDED: &str = "superseded";

impl PreviewHandle {
    pub(crate) fn render(
        &self,
        request: lvgl_sim::scene::SceneRenderRequest,
    ) -> Result<Vec<u8>, String> {
        let (reply, receive) = mpsc::channel();
        self.sender
            .send(PreviewJob { request, reply })
            .map_err(|_| "preview thread gone".to_string())?;
        receive
            .recv()
            .map_err(|_| "preview thread gone".to_string())?
    }
}
