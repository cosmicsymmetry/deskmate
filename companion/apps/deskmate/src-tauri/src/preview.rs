//! A dedicated thread that owns the one process-wide [`lvgl_sim::Simulator`] and
//! renders card previews for the settings webview.
//!
//! `Simulator` is not `Sync` (see its own docs) and must never migrate between
//! threads, so this module claims exactly one `Simulator` for the process lifetime
//! and drives it from a single dedicated thread. Requests arrive on an `mpsc`
//! channel with latest-wins coalescing: if more than one render is queued when the
//! thread wakes up, only the newest is rendered and every superseded request is told
//! so explicitly (spec 2.5) rather than left to wait for a stale answer.

use std::sync::mpsc;

pub struct PreviewHandle {
    sender: mpsc::Sender<PreviewJob>,
}

struct PreviewJob {
    request: lvgl_sim::RenderRequest,
    reply: mpsc::Sender<Result<Vec<u8>, String>>,
}

pub fn spawn() -> PreviewHandle {
    let (sender, receiver) = mpsc::channel::<PreviewJob>();
    std::thread::Builder::new()
        .name("preview-sim".into())
        .spawn(move || {
            let mut simulator = match lvgl_sim::Simulator::new() {
                Ok(simulator) => simulator,
                Err(error) => {
                    // Drain forever with the error: "preview unavailable",
                    // never a stale or invented frame (spec §3.3).
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
                    let _ = job.reply.send(Err("superseded".into()));
                    job = newer;
                }
                let result = simulator
                    .render_png(&job.request)
                    .map_err(|error| format!("render failed: {error:?}"));
                let _ = job.reply.send(result);
            }
        })
        .expect("spawn preview thread");
    PreviewHandle { sender }
}

impl PreviewHandle {
    pub fn render(&self, request: lvgl_sim::RenderRequest) -> Result<Vec<u8>, String> {
        let (reply, receive) = mpsc::channel();
        self.sender
            .send(PreviewJob { request, reply })
            .map_err(|_| "preview thread gone".to_string())?;
        receive
            .recv()
            .map_err(|_| "preview thread gone".to_string())?
    }
}
