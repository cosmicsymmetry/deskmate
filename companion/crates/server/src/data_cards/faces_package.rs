//! The faces package, seen from the server: a program that speaks two verbs.
//!
//! The faces themselves -- what they fetch and how they draw -- live in
//! `companion/faces/` as TypeScript. The server runs that package as a subprocess:
//!
//! - `describe` prints the catalog the browser's add menu and settings form are
//!   built from;
//! - `render` reads `{"kind", "settings"}` on stdin and writes one PNG to stdout,
//!   which goes through [`crate::image_ingest::canonical_frame_from_png`] exactly
//!   as an external producer's POST would. The server is just another producer.
//!
//! # Exit codes are the error taxonomy
//!
//! 0 is a frame. 2 means the owner must change a setting and a retry cannot help.
//! Anything else is transient. Either way the stored frame is kept.
//!
//! # What the child may not have
//!
//! The environment is cleared. The server's own environment carries
//! `DESKMATE_ADMIN_TOKEN`, and a process whose job is to fetch arbitrary feed
//! URLs has no business holding it.
//!
//! # Nothing here waits unboundedly
//!
//! A child that never answers is killed at the deadline, and the pipe readers are
//! never joined: they collect into shared buffers the caller takes once the child
//! has exited, so a stray process holding a pipe open cannot wedge the caller.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::FaceFieldOption;

/// A face fetches over the network, so this is generous; it bounds a hung child,
/// not a slow API -- the package applies its own tighter request timeout.
const RENDER_TIMEOUT: Duration = Duration::from_secs(45);
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(15);
/// The ingest route's own body limit: a face may not be larger than a producer's.
const MAX_PNG_BYTES: usize = 1024 * 1024;
const MAX_CATALOG_BYTES: usize = 256 * 1024;
const MAX_STDERR_BYTES: usize = 8 * 1024;
const MAX_MESSAGE_CHARS: usize = 200;
const POLL: Duration = Duration::from_millis(10);
/// How long an exited child's pipes are given to reach EOF before being read anyway.
const EOF_GRACE: Duration = Duration::from_millis(250);
const EXIT_CONFIGURATION: i32 = 2;

/// How to run the faces package. The verb is appended to `arguments`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceCommand {
    program: PathBuf,
    arguments: Vec<OsString>,
}

impl FaceCommand {
    /// `bun run <faces_dir>/src/main.ts`, the production shape.
    #[must_use]
    pub fn bun(bun: PathBuf, faces_dir: &Path) -> Self {
        Self {
            program: bun,
            arguments: vec!["run".into(), faces_dir.join("src/main.ts").into()],
        }
    }

    /// Any program that speaks the two verbs. The test suites use a shell script.
    #[must_use]
    pub fn program(program: PathBuf) -> Self {
        Self {
            program,
            arguments: Vec::new(),
        }
    }
}

/// One face the package can draw, as `describe` states it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct CatalogFace {
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) fields: Vec<CatalogField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum CatalogField {
    Text {
        key: String,
        label: String,
        placeholder: String,
        #[serde(default)]
        default: String,
    },
    Url {
        key: String,
        label: String,
        placeholder: String,
        #[serde(default)]
        default: String,
    },
    Enum {
        key: String,
        label: String,
        default: String,
        options: Vec<FaceFieldOption>,
    },
}

impl CatalogField {
    pub(crate) fn key(&self) -> &str {
        match self {
            Self::Text { key, .. } | Self::Url { key, .. } | Self::Enum { key, .. } => key,
        }
    }

    pub(crate) fn default_value(&self) -> &str {
        match self {
            Self::Text { default, .. } | Self::Url { default, .. } | Self::Enum { default, .. } => {
                default
            }
        }
    }
}

/// Why a face produced no frame.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FaceRenderError {
    /// The owner must change a setting; retrying cannot help.
    #[error("{0}")]
    Configuration(String),
    /// The world did not cooperate this time.
    #[error("{0}")]
    Transient(String),
}

struct Finished {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: String,
}

/// What a pipe reader has collected so far. Shared rather than returned, because
/// the caller must be able to take it without waiting for the pipe to reach EOF.
#[derive(Default)]
struct Collected {
    bytes: Vec<u8>,
    overflowed: bool,
}

/// Reads up to `cap` bytes into `collected`, then keeps draining so the child is
/// never blocked on a full pipe. Signals `finished` at EOF.
fn drain(
    mut pipe: impl std::io::Read + Send + 'static,
    cap: usize,
) -> (Arc<Mutex<Collected>>, mpsc::Receiver<()>) {
    let collected = Arc::new(Mutex::new(Collected::default()));
    let (finished_sender, finished) = mpsc::channel();
    let shared = Arc::clone(&collected);
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 16 * 1024];
        while let Ok(read) = pipe.read(&mut buffer) {
            if read == 0 {
                break;
            }
            let mut collected = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let room = cap.saturating_sub(collected.bytes.len());
            collected.bytes.extend_from_slice(&buffer[..read.min(room)]);
            collected.overflowed |= read > room;
        }
        let _ = finished_sender.send(());
    });
    (collected, finished)
}

fn take(collected: &Mutex<Collected>) -> Collected {
    std::mem::take(
        &mut *collected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

fn run(
    command: &FaceCommand,
    verb: &str,
    stdin: &[u8],
    stdout_cap: usize,
    timeout: Duration,
) -> Result<Finished, String> {
    let mut child = Command::new(&command.program)
        .args(&command.arguments)
        .arg(verb)
        .env_clear()
        // `bun` must find itself and nothing else; no transpiler cache, because the
        // unit's home is not writable and a cache miss costs milliseconds.
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("BUN_RUNTIME_TRANSPILER_CACHE_PATH", "0")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", command.program.display()))?;

    // The request is a few hundred bytes, far below a pipe buffer, so this cannot
    // block on a child that is not reading yet. Dropping it closes the pipe.
    if let Some(mut pipe) = child.stdin.take() {
        let _ = pipe.write_all(stdin);
    }
    let stdout = child.stdout.take().map(|pipe| drain(pipe, stdout_cap));
    let stderr = child
        .stderr
        .take()
        .map(|pipe| drain(pipe, MAX_STDERR_BYTES));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "the faces package did not answer within {}s",
                    timeout.as_secs()
                ));
            }
            Err(error) => return Err(format!("could not wait for the faces package: {error}")),
        }
    };

    // The child has exited, so everything it wrote is already in the pipe and the
    // readers reach EOF at once -- normally. EOF also needs every OTHER holder of
    // the write end to be gone, and an unrelated process can inherit one (on macOS
    // a pipe gains CLOEXEC non-atomically, so a concurrent fork elsewhere in this
    // process races it). So EOF is waited for briefly and never required: what has
    // been collected from an exited child is complete either way.
    let mut output = [Collected::default(), Collected::default()];
    for (slot, reader) in output.iter_mut().zip([stdout, stderr]) {
        if let Some((collected, finished)) = reader {
            let _ = finished.recv_timeout(EOF_GRACE);
            *slot = take(&collected);
        }
    }
    let [stdout, stderr] = output;
    if stdout.overflowed {
        return Err(format!(
            "the faces package wrote more than {stdout_cap} bytes"
        ));
    }
    Ok(Finished {
        code: status.code(),
        stdout: stdout.bytes,
        stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
    })
}

/// The last line the package wrote to stderr, bounded, for the log.
fn message_of(finished: &Finished) -> String {
    let line = finished
        .stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("the faces package gave no reason")
        .trim();
    line.chars().take(MAX_MESSAGE_CHARS).collect()
}

/// Asks the package which faces it can draw.
pub(crate) fn describe(command: &FaceCommand) -> Result<Vec<CatalogFace>, String> {
    let finished = run(
        command,
        "describe",
        &[],
        MAX_CATALOG_BYTES,
        DESCRIBE_TIMEOUT,
    )?;
    if finished.code != Some(0) {
        return Err(format!("describe failed: {}", message_of(&finished)));
    }
    let mut catalog: Vec<CatalogFace> = serde_json::from_slice(&finished.stdout)
        .map_err(|error| format!("the face catalog is not the expected JSON: {error}"))?;
    // A duplicated kind would make "which face is this spec" ambiguous; the first
    // one wins and the rest are dropped rather than failing the whole catalog.
    let mut seen = std::collections::HashSet::new();
    catalog.retain(|face| !face.kind.trim().is_empty() && seen.insert(face.kind.clone()));
    Ok(catalog)
}

/// Fetches and draws one face, returning the PNG the package produced.
pub(crate) fn render(
    command: &FaceCommand,
    kind: &str,
    settings: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<Vec<u8>, FaceRenderError> {
    let request = serde_json::json!({ "kind": kind, "settings": settings }).to_string();
    let finished = run(
        command,
        "render",
        request.as_bytes(),
        MAX_PNG_BYTES,
        RENDER_TIMEOUT,
    )
    .map_err(FaceRenderError::Transient)?;
    match finished.code {
        Some(0) if finished.stdout.is_empty() => Err(FaceRenderError::Transient(
            "the faces package exited cleanly without a frame".into(),
        )),
        Some(0) => Ok(finished.stdout),
        Some(EXIT_CONFIGURATION) => Err(FaceRenderError::Configuration(message_of(&finished))),
        _ => Err(FaceRenderError::Transient(message_of(&finished))),
    }
}

#[cfg(test)]
pub(crate) fn fake() -> FaceCommand {
    FaceCommand::program(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake-faces.sh"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn steer(word: &str) -> BTreeMap<String, serde_json::Value> {
        BTreeMap::from([(
            "location".to_owned(),
            serde_json::Value::String(word.to_owned()),
        )])
    }

    #[test]
    fn the_catalog_is_read_with_its_defaults() {
        let catalog = describe(&fake()).expect("the fake package describes itself");
        let kinds: Vec<&str> = catalog.iter().map(|face| face.kind.as_str()).collect();
        assert_eq!(kinds, ["weather", "rss", "token", "headlines"]);
        let token = &catalog[2];
        assert_eq!(
            token.fields[0].default_value(),
            "",
            "an absent default is blank"
        );
        assert_eq!(token.fields[1].default_value(), "usd");
    }

    #[test]
    fn a_frame_comes_back_as_the_bytes_the_package_wrote() {
        let png = render(&fake(), "weather", &steer("Dubai")).expect("a frame");
        assert!(png.starts_with(b"\x89PNG"));
        assert!(crate::image_ingest::canonical_frame_from_png(&png).is_ok());
    }

    #[test]
    fn exit_two_is_the_owners_to_fix_and_anything_else_is_transient() {
        assert_eq!(
            render(&fake(), "token", &steer("refuse-as-configuration")),
            Err(FaceRenderError::Configuration(
                "the coin was not found; check the coin ID".into()
            ))
        );
        assert_eq!(
            render(&fake(), "token", &steer("refuse-as-transient")),
            Err(FaceRenderError::Transient(
                "api.example returned HTTP 503".into()
            ))
        );
    }

    #[test]
    fn a_missing_program_is_transient_rather_than_a_panic() {
        let nowhere = FaceCommand::bun("/nonexistent/bun".into(), Path::new("/nonexistent"));
        assert!(matches!(
            render(&nowhere, "weather", &BTreeMap::new()),
            Err(FaceRenderError::Transient(message)) if message.contains("could not start")
        ));
        assert!(describe(&nowhere).is_err());
    }

    #[test]
    fn a_flood_on_stdout_is_refused_at_the_cap_not_buffered() {
        assert!(matches!(
            render(&fake(), "weather", &steer("answer-with-a-flood")),
            Err(FaceRenderError::Transient(message)) if message.contains("more than")
        ));
    }

    #[test]
    fn a_child_that_never_answers_is_killed_at_the_deadline() {
        let started = Instant::now();
        let outcome = run(
            &fake(),
            "render",
            br#"{"x":"never-answer"}"#,
            1024,
            Duration::from_millis(150),
        );
        assert!(outcome.is_err_and(|message| message.contains("did not answer")));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the wait is bounded"
        );
    }

    #[test]
    fn the_child_never_sees_the_servers_environment() {
        // The server's environment holds DESKMATE_ADMIN_TOKEN. HOME stands in for it
        // here: it is set in this process, and must be absent in the child.
        assert!(
            std::env::var_os("HOME").is_some(),
            "the canary is present here"
        );
        let finished = run(
            &fake(),
            "render",
            br#"{"x":"echo-the-environment"}"#,
            1024,
            Duration::from_secs(5),
        )
        .expect("the fake runs");
        assert!(
            finished.stderr.contains("PATH="),
            "the child printed its environment"
        );
        assert!(
            !finished.stderr.contains("HOME="),
            "and the server's was not inherited"
        );
    }
}
