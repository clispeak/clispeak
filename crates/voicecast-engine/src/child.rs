//! A spawned process that can be waited on and killed from another thread.
//!
//! Every engine here works by spawning something — Piper, a player, espeak-ng
//! — and both of the things an engine must do to that process are awkward at
//! once. Waiting needs `&mut Child`, so it wants a lock; killing needs
//! `&mut Child` too, from a *different* thread, while the wait is in progress.
//!
//! Holding one lock across the wait is the obvious arrangement and it was
//! wrong: `stop` then parked until playback finished and killed nothing, so
//! "stop immediately, mid-sentence" was not true of any engine that spawns
//! (issue #58). The lock here is held only for the instant a `try_wait` or a
//! `kill` takes, and the waiting is done by polling between those instants.
//!
//! Polling rather than blocking costs a wakeup every [`POLL`] and buys a
//! `stop` that lands within that. Blocking would need a portable way to kill
//! a process by handle from another thread, which `std` does not offer and
//! which would mean platform code in three places instead of none.

use std::io::Read;
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::EngineError;

/// How often a wait checks whether the process has finished.
///
/// The upper bound on how long `stop` takes to cut audio. Ten milliseconds is
/// far below what anyone can hear as a delay, and a hundred wakeups a second
/// against a process that is busy synthesising speech is not measurable.
const POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// The most stderr kept for an error message.
///
/// A failing player can produce output without bound — a device that is gone
/// can be complained about once per buffer — so the tail is capped rather
/// than the whole of it held. The tail, not the head: the last thing a
/// process says before dying is the part worth reading.
const STDERR_TAIL: usize = 1024;

/// A running process, waitable and killable from different threads.
pub(crate) struct Running {
    /// What to call it in an error. `piper`, `afplay`, `espeak-ng`.
    label: String,
    child: Mutex<Child>,
    /// Set before the kill, so a wait that loses the race reports the stop it
    /// was asked for rather than a failure. Deliberately stopping a process
    /// makes it exit non-zero, and without this every interrupt would be
    /// reported as the player having crashed.
    killed: AtomicBool,
    /// Drains stderr on its own thread.
    ///
    /// Reading it only after the process exits would deadlock the moment the
    /// output exceeds the pipe buffer: the child blocks writing, so it never
    /// exits, so nothing ever reads. That is a hang rather than a lost
    /// message, which is worse than the problem being solved.
    stderr: Mutex<Option<std::thread::JoinHandle<String>>>,
    /// The drained tail, once the thread has been joined.
    tail: Mutex<Option<String>>,
    /// How it ended, once known.
    ///
    /// Cached because a process is waited on more than once: the file-based
    /// path waits for synthesis before it can play the file, and `finish`
    /// then asks about the same process again. A second `try_wait` on a
    /// reaped child does not return the status again — on Unix it fails with
    /// `ECHILD` — so without this the second ask invents an error for a
    /// process that exited cleanly.
    status: Mutex<Option<ExitStatus>>,
}

impl Running {
    /// Take ownership of a spawned child, draining its stderr.
    pub(crate) fn new(label: impl Into<String>, mut child: Child) -> Arc<Self> {
        let stderr = child.stderr.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut tail = Vec::new();
                let mut buf = [0u8; 512];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    tail.extend_from_slice(&buf[..n]);
                    if tail.len() > STDERR_TAIL * 2 {
                        tail.drain(..tail.len() - STDERR_TAIL);
                    }
                }
                if tail.len() > STDERR_TAIL {
                    tail.drain(..tail.len() - STDERR_TAIL);
                }
                String::from_utf8_lossy(&tail).trim().to_string()
            })
        });

        Arc::new(Self {
            label: label.into(),
            child: Mutex::new(child),
            killed: AtomicBool::new(false),
            stderr: Mutex::new(stderr),
            tail: Mutex::new(None),
            status: Mutex::new(None),
        })
    }

    /// Wait for the process, without holding anything [`kill`] needs.
    ///
    /// [`kill`]: Running::kill
    pub(crate) fn wait(&self) -> Result<(), EngineError> {
        if let Some(status) = *self.status.lock().expect("status lock") {
            return self.check(status);
        }
        let status = loop {
            let polled = self
                .child
                .lock()
                .expect("child lock")
                .try_wait()
                .map_err(|e| {
                    EngineError::Unavailable(format!("waiting for {}: {e}", self.label))
                })?;
            match polled {
                Some(status) => break status,
                // Nothing is held across this sleep, which is the whole point.
                None => std::thread::sleep(POLL),
            }
        };
        *self.status.lock().expect("status lock") = Some(status);
        self.check(status)
    }

    /// Turn an exit status into a result.
    ///
    /// A process that ran and failed used to be indistinguishable from one
    /// that spoke: `wait()` returning `Ok` was taken as the audio having been
    /// heard, when it only means the process was successfully waited for.
    /// Issue #59.
    fn check(&self, status: ExitStatus) -> Result<(), EngineError> {
        // Asked for, so not a fault. The queue also checks its own cut before
        // the engine's error for this reason; this makes the engine honest on
        // its own rather than relying on the caller to know.
        if self.killed.load(Ordering::SeqCst) || status.success() {
            return Ok(());
        }
        Err(EngineError::Failed {
            command: self.label.clone(),
            // `code()` is `None` when a signal ended it, which is worth
            // distinguishing: "killed by a signal" and "exited 1" are
            // different problems and the second is the one a user can act on.
            code: match status.code() {
                Some(code) => format!("exit code {code}"),
                None => "a signal".to_string(),
            },
            detail: self.stderr_tail().filter(|s| !s.is_empty()),
        })
    }

    /// Whatever the process last wrote to stderr, once it has exited.
    ///
    /// Joined at most once and then remembered, so asking twice does not lose
    /// the answer the second time.
    fn stderr_tail(&self) -> Option<String> {
        let mut tail = self.tail.lock().expect("tail lock");
        if tail.is_none()
            && let Some(handle) = self.stderr.lock().expect("stderr lock").take()
        {
            *tail = handle.join().ok();
        }
        tail.clone()
    }

    /// End the process now, and reap it.
    ///
    /// Reaping matters: `kill` without a `wait` left one zombie per interrupt
    /// for the lifetime of the daemon, and an interrupt is a thing a person
    /// does repeatedly.
    pub(crate) fn kill(&self) {
        self.killed.store(true, Ordering::SeqCst);
        let mut child = self.child.lock().expect("child lock");
        let _ = child.kill();
        // Reaping matters: a `kill` without this left one zombie per
        // interrupt for the lifetime of the daemon.
        if let Ok(status) = child.wait() {
            *self.status.lock().expect("status lock") = Some(status);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn spawn(program: &str, args: &[&str]) -> Arc<Running> {
        let child = Command::new(program)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        Running::new(program, child)
    }

    /// Issue #58, and the reason a `FakeEngine` could not catch it.
    ///
    /// The old arrangement held one lock across the wait, so a `kill` from
    /// another thread parked until the process ended on its own. With a
    /// thirty-second child that is a thirty-second `stop`.
    #[test]
    fn a_kill_lands_while_the_process_is_still_running() {
        let running = spawn("sleep", &["30"]);

        let waiter = {
            let running = Arc::clone(&running);
            std::thread::spawn(move || running.wait())
        };
        // Let the waiter get into its loop, so this is the contended case
        // rather than a kill that happens to arrive first.
        std::thread::sleep(Duration::from_millis(50));

        let started = Instant::now();
        running.kill();
        let took = started.elapsed();

        assert!(
            took < Duration::from_secs(2),
            "stop took {took:?} against a child that had 30s left to run"
        );
        let outcome = waiter.join().expect("waiter");
        assert!(
            outcome.is_ok(),
            "a process we killed on purpose is not a failure: {outcome:?}"
        );
    }

    /// Issue #59. `wait()` succeeding means the process was waited for, not
    /// that it worked.
    #[test]
    fn a_process_that_exits_non_zero_is_an_error() {
        let running = spawn("sh", &["-c", "echo 'no such device' >&2; exit 3"]);
        let err = running.wait().expect_err("exit 3 is not success");
        match err {
            EngineError::Failed {
                command,
                code,
                detail,
            } => {
                assert_eq!(command, "sh");
                assert_eq!(code, "exit code 3");
                assert_eq!(
                    detail.as_deref(),
                    Some("no such device"),
                    "the reason it gave is the whole point of capturing stderr"
                );
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    /// A failure with nothing to say still names the command and the code.
    #[test]
    fn a_silent_failure_still_reports_the_command() {
        let running = spawn("sh", &["-c", "exit 1"]);
        let err = running.wait().expect_err("exit 1 is not success");
        assert!(
            matches!(err, EngineError::Failed { detail: None, .. }),
            "{err:?}"
        );
        assert!(err.reason().contains("sh"), "{}", err.reason());
        assert!(err.reason().contains("exit code 1"), "{}", err.reason());
    }

    /// The double-wait that the file-based path performs.
    ///
    /// Synthesis is waited for so the file exists, and `finish` then asks
    /// about the same process again. A second `try_wait` on a reaped child
    /// fails with `ECHILD`, which would invent an error for a clean exit.
    #[test]
    fn waiting_twice_gives_the_same_answer() {
        let running = spawn("sh", &["-c", "exit 0"]);
        assert!(running.wait().is_ok());
        assert!(running.wait().is_ok(), "the second ask must not invent one");

        let failed = spawn("sh", &["-c", "exit 4"]);
        assert!(failed.wait().is_err());
        let second = failed.wait().expect_err("still a failure");
        assert!(
            second.reason().contains("exit code 4"),
            "{}",
            second.reason()
        );
    }

    /// Success is success, and says nothing.
    #[test]
    fn a_clean_exit_is_ok() {
        assert!(spawn("true", &[]).wait().is_ok());
    }
}
