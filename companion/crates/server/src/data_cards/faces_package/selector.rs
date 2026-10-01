//! A selector-only process. Render never acquires this lock or uses this pipe.
//! Requests and replies are bounded JSON lines; no account directory or secrets
//! reach the worker. A failed/older worker falls back to the one-shot `tap` verb.

use super::*;
use std::io::{BufRead as _, BufReader, Read as _};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};

const REAP_INTERVAL: Duration = Duration::from_millis(100);

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_REQUESTS: usize = 256;
const MAX_AGE: Duration = Duration::from_secs(60);

#[derive(Debug, Default)]
pub(super) struct Selector {
    retained: Mutex<Retained>,
}

#[derive(Debug, Default)]
struct Retained {
    worker: Option<Worker>,
    retry_after: Option<Instant>,
}

#[derive(Debug)]
struct Worker {
    child: Arc<Mutex<Option<Child>>>,
    exited: Arc<AtomicBool>,
    #[cfg(test)]
    io_thread: std::thread::JoinHandle<()>,
    requests: mpsc::SyncSender<Vec<u8>>,
    responses: mpsc::Receiver<Result<Vec<u8>, String>>,
    started: Instant,
    used: usize,
    retiring: bool,
}

#[derive(Deserialize)]
struct Answer {
    code: i32,
    result: Option<TapSelection>,
    error: Option<String>,
    #[serde(default)]
    retire: bool,
}

impl Selector {
    /// `None` means use a fresh, isolated one-shot process. A worker's semantic
    /// refusal is returned normally; only a broken transport disables reuse.
    pub(super) fn tap(
        &self,
        command: &FaceCommand,
        input: &[u8],
    ) -> Option<Result<TapSelection, FaceRenderError>> {
        if input.len() > MAX_REQUEST_BYTES {
            return None;
        }
        let mut retained = self
            .retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if retained.retry_after.is_some_and(|at| Instant::now() < at) {
            return None;
        }
        if retained.worker.as_ref().is_some_and(|worker| {
            worker.started.elapsed() >= MAX_AGE || worker.used >= MAX_REQUESTS || worker.retiring
        }) {
            retained.worker = None;
        }
        if retained.worker.is_none() {
            retained.worker = Worker::start(command).ok();
        }
        let answer = retained
            .worker
            .as_mut()
            .and_then(|worker| worker.exchange(input).ok());
        if let Some(answer) = answer {
            Some(answer)
        } else {
            retained.worker = None;
            // An independently deployed old package must not pay a failed
            // worker launch on every tap. Retry within the catalog cadence.
            retained.retry_after = Some(Instant::now() + MAX_AGE);
            None
        }
    }
}

impl Worker {
    fn start(command: &FaceCommand) -> Result<Self, String> {
        let mut child = builder(command, "tap-worker", None)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        let (requests, receiver) = mpsc::sync_channel::<Vec<u8>>(1);
        let (sender, responses) = mpsc::sync_channel(1);
        let child = Arc::new(Mutex::new(Some(child)));
        let exited = Arc::new(AtomicBool::new(false));
        let weak_child = Arc::downgrade(&child);
        let reaped = Arc::clone(&exited);
        std::thread::spawn(move || {
            loop {
                let Some(child) = weak_child.upgrade() else {
                    break;
                };
                let done = {
                    let mut child = child
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if child
                        .as_mut()
                        .is_none_or(|child| matches!(child.try_wait(), Ok(Some(_))))
                    {
                        *child = None;
                        true
                    } else {
                        false
                    }
                };
                if done {
                    reaped.store(true, Ordering::Release);
                    break;
                }
                drop(child);
                std::thread::sleep(REAP_INTERVAL);
            }
        });
        let io_exited = Arc::clone(&exited);
        // The caller owns the deadline and kills the child. Never join this IO
        // thread: an inherited pipe writer must not turn a crash into a hang.
        let io_thread = std::thread::spawn(move || {
            while !io_exited.load(Ordering::Acquire) {
                let input = match receiver.recv_timeout(REAP_INTERVAL) {
                    Ok(input) => input,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let answer = (|| {
                    stdin.write_all(&input)?;
                    stdin.write_all(b"\n")?;
                    stdin.flush()?;
                    let mut bytes = Vec::new();
                    stdout
                        .by_ref()
                        .take((MAX_PLAN_BYTES + 1) as u64)
                        .read_until(b'\n', &mut bytes)?;
                    Ok::<_, std::io::Error>(bytes)
                })()
                .map_err(|error| error.to_string());
                if sender.send(answer).is_err() {
                    break;
                }
            }
        });
        #[cfg(not(test))]
        drop(io_thread); // Detach: inherited pipe handles must never make teardown join.
        Ok(Self {
            child,
            exited,
            #[cfg(test)]
            io_thread,
            requests,
            responses,
            started: Instant::now(),
            used: 0,
            retiring: false,
        })
    }

    fn exchange(&mut self, input: &[u8]) -> Result<Result<TapSelection, FaceRenderError>, String> {
        self.exchange_with_timeout(input, PLAN_TIMEOUT)
    }

    fn exchange_with_timeout(
        &mut self,
        input: &[u8],
        timeout: Duration,
    ) -> Result<Result<TapSelection, FaceRenderError>, String> {
        self.requests
            .try_send(input.to_vec())
            .map_err(|error| error.to_string())?;
        let bytes = self
            .responses
            .recv_timeout(timeout)
            .map_err(|error| error.to_string())??;
        if bytes.len() > MAX_PLAN_BYTES || !bytes.ends_with(b"\n") {
            return Err("unframed or oversized selector response".into());
        }
        let answer: Answer = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        self.used += 1;
        self.retiring = answer.retire;
        if answer.code == 0 {
            Ok(Ok(answer.result.ok_or("selector omitted its result")?))
        } else {
            let message = answer
                .error
                .unwrap_or_else(|| "selector refused the tap".into());
            Ok(Err(if answer.code == EXIT_CONFIGURATION {
                FaceRenderError::Configuration(message)
            } else {
                FaceRenderError::Transient(message)
            }))
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.exited.store(true, Ordering::Release);
        if let Some(mut child) = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn command(script: &str) -> (tempfile::TempDir, FaceCommand) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("selector");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut command = FaceCommand::program(path);
        command.selector = Some(Arc::new(Selector::default()));
        (dir, command)
    }

    const ECHO: &str = r#"
if [ "$1" = tap-worker ]; then
    n=0
    while IFS= read -r line; do
        n=$((n+1))
        printf '{"code":0,"result":{"view":"%s:%s:%s:%s"}}\n' "$$" "$n" "${HOME-unset}" "${DESKMATE_CONFIG_DIR-unset}"
    done
else
    cat >/dev/null
    printf '{"view":"fresh"}'
fi
"#;

    fn select(command: &FaceCommand) -> String {
        super::super::tap(command, "rss", &BTreeMap::new(), None, 1)
            .unwrap()
            .view
    }

    #[test]
    fn selector_reuses_a_process_without_inheriting_environment_or_account_directory() {
        let (_dir, command) = command(ECHO);
        let first = select(&command);
        let second = select(&command.clone());
        assert!(first.ends_with(":1:unset:unset"), "{first}");
        assert!(second.ends_with(":2:unset:unset"), "{second}");
        assert_eq!(first.split(':').next(), second.split(':').next());
    }

    #[test]
    fn a_warm_selector_never_waits_for_a_render_using_the_same_command() {
        let (dir, command) = command(ECHO);
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = render ]; then\n: > '{}'\nwhile [ ! -e '{}' ]; do sleep 0.01; done\nexit 1\nfi\n{ECHO}",
            dir.path().join("blocked").display(),
            dir.path().join("release").display()
        );
        std::fs::write(&command.program, script).unwrap();
        select(&command);
        let render_command = command.clone();
        let render = std::thread::spawn(move || {
            run(
                &render_command,
                "render",
                b"{}",
                MAX_PNG_BYTES,
                RENDER_TIMEOUT,
                Some(Path::new("/account")),
            )
        });
        let started = Instant::now();
        while !dir.path().join("blocked").exists() {
            assert!(started.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(5));
        }
        let selector_command = command.clone();
        let (answer, received) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = answer.send(select(&selector_command));
        });
        let selected = received.recv_timeout(Duration::from_secs(1));
        assert!(!render.is_finished());
        std::fs::write(dir.path().join("release"), b"").unwrap();
        render.join().unwrap().unwrap();
        assert!(selected.unwrap().ends_with(":2:unset:unset"));
    }

    #[test]
    fn an_idle_exited_worker_is_reaped_and_its_io_thread_stops_without_another_tap() {
        let (_dir, command) = command("exit 0");
        let worker = Worker::start(&command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !(worker.child.lock().unwrap().is_none() && worker.io_thread.is_finished()) {
            assert!(
                Instant::now() < deadline,
                "idle child or its IO thread was retained"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        // Keep the Worker alive throughout: replacement/drop cannot satisfy this test.
        assert!(worker.exited.load(Ordering::Acquire));
    }

    #[test]
    fn a_crashed_worker_falls_back_to_a_fresh_spawn_and_can_recover() {
        let (_dir, command) = command(ECHO);
        select(&command);
        let selector = command.selector.as_ref().unwrap();
        {
            let mut retained = selector.retained.lock().unwrap();
            let worker = retained.worker.as_mut().unwrap();
            let mut child = worker.child.lock().unwrap();
            let child = child.as_mut().unwrap();
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert_eq!(select(&command), "fresh");
        assert_eq!(
            select(&command),
            "fresh",
            "cooldown uses the compatible verb"
        );
        selector.retained.lock().unwrap().retry_after = Some(Instant::now());
        assert!(select(&command).ends_with(":1:unset:unset"));
    }

    #[test]
    fn request_count_and_age_each_retire_the_worker_and_reload_the_package() {
        let (_dir, command) = command(ECHO);
        let first = select(&command);
        let selector = command.selector.as_ref().unwrap();
        selector
            .retained
            .lock()
            .unwrap()
            .worker
            .as_mut()
            .unwrap()
            .used = MAX_REQUESTS;
        let second = select(&command);
        assert_ne!(first.split(':').next(), second.split(':').next());
        std::fs::write(&command.program, "#!/bin/sh\nwhile IFS= read -r line; do printf '{\"code\":0,\"result\":{\"view\":\"updated\"}}\\n'; done\n").unwrap();
        selector
            .retained
            .lock()
            .unwrap()
            .worker
            .as_mut()
            .unwrap()
            .started = Instant::now().checked_sub(MAX_AGE).unwrap();
        assert_eq!(
            select(&command),
            "updated",
            "faces-only update needs no restart"
        );
    }

    #[test]
    fn oversized_requests_never_enter_the_retained_worker() {
        let (_dir, command) = command(ECHO);
        assert!(
            command
                .selector
                .as_ref()
                .unwrap()
                .tap(&command, &vec![b'x'; MAX_REQUEST_BYTES + 1])
                .is_none()
        );
    }

    #[test]
    fn a_complete_json_line_one_byte_over_the_response_cap_is_refused() {
        let empty = "{\"code\":0,\"result\":{\"view\":\"\"}}\n";
        let output = empty.replace(
            "\"view\":\"\"",
            &format!(
                "\"view\":\"{}\"",
                "x".repeat(MAX_PLAN_BYTES + 1 - empty.len())
            ),
        );
        assert_eq!(output.len(), MAX_PLAN_BYTES + 1);
        let (_dir, command) = command(&format!("read line; printf '%s' '{output}'"));
        let mut worker = Worker::start(&command).unwrap();
        assert!(worker.exchange(b"{}").is_err());
    }

    #[test]
    fn worker_refusals_preserve_configuration_and_transient_errors() {
        for (code, expected) in [
            (2, FaceRenderError::Configuration("refused".into())),
            (1, FaceRenderError::Transient("refused".into())),
        ] {
            let (_dir, command) = command(&format!(
                "while IFS= read -r line; do printf '{{\"code\":{code},\"error\":\"refused\"}}\\n'; done"
            ));
            assert_eq!(
                super::super::tap(&command, "rss", &BTreeMap::new(), None, 1),
                Err(expected)
            );
        }
    }

    #[test]
    fn a_worker_retiring_at_its_memory_budget_is_replaced_without_a_cooldown() {
        let (_dir, command) = command(
            "read line; printf '{\"code\":0,\"result\":{\"view\":\"%s\"},\"retire\":true}\\n' \"$$\"",
        );
        let first = select(&command);
        let second = select(&command);
        assert_ne!(first, second);
        assert!(
            command
                .selector
                .as_ref()
                .unwrap()
                .retained
                .lock()
                .unwrap()
                .retry_after
                .is_none()
        );
    }

    #[test]
    fn valid_json_without_a_line_terminator_is_refused() {
        let (_dir, command) =
            command("read line; printf '{\"code\":0,\"result\":{\"view\":\"unterminated\"}}'");
        let mut worker = Worker::start(&command).unwrap();
        assert_eq!(
            worker.exchange(b"{}").unwrap_err(),
            "unframed or oversized selector response"
        );
    }

    #[test]
    fn oversized_or_unframed_replies_and_blocked_pipes_are_bounded() {
        for script in [
            "read line; printf '{\"code\":0,\"result\":{\"view\":\"'; head -c 20000 /dev/zero | tr '\\000' x; printf '\"}}\\n'",
            "read line; printf '{\"code\":0,\"result\":{\"view\":\"unterminated\"}}'",
            "sleep 2 & wait",
        ] {
            let (_dir, command) = command(script);
            let mut worker = Worker::start(&command).unwrap();
            let started = Instant::now();
            assert!(
                worker
                    .exchange_with_timeout(b"{}", Duration::from_millis(100))
                    .is_err()
            );
            drop(worker);
            assert!(started.elapsed() < Duration::from_secs(1));
        }
    }

    #[test]
    fn one_shot_exit_does_not_wait_for_an_inherited_pipe_to_close() {
        let (_dir, command) = command("cat >/dev/null; sleep 5 & printf '{\"view\":\"fresh\"}'");
        let started = Instant::now();
        let finished = run(&command, "tap", b"{}", MAX_PLAN_BYTES, PLAN_TIMEOUT, None).unwrap();
        assert_eq!(finished.code, Some(0));
        assert_eq!(finished.stdout, br#"{"view":"fresh"}"#);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "EOF must not wait for sleep"
        );
    }
}
