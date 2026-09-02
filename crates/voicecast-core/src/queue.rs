//! The speaking queue and the thread that drains it.
//!
//! A device speaks one message at a time, so everything that wants to be
//! heard queues here. Priority decides what happens when something arrives
//! while another message is already playing — see `docs/cli.md`.
//!
//! Speaking blocks, so this owns a dedicated thread rather than starving the
//! async runtime that is accepting connections.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use voicecast_engine::SpeechEngine;
use voicecast_proto::Status;

/// A message waiting its turn.
pub struct Job {
    /// Identifies the message, for control commands and reports.
    pub msg_id: String,
    /// The chunks to speak, in order.
    pub chunks: Vec<String>,
    /// A voice to use for this message alone, if this device has it.
    pub voice: Option<String>,
    /// Signalled when speaking ends, for callers that asked to wait.
    ///
    /// Optional because the common case is fire-and-forget: an agent firing
    /// notifications should not pay for machinery it is not using.
    pub done: Option<tokio::sync::oneshot::Sender<Status>>,
}

impl Job {
    /// Tell a waiting caller how this ended, if anyone is still listening.
    fn finish(self, status: Status) {
        if let Some(done) = self.done {
            // Ignored deliberately: the caller may have stopped waiting.
            let _ = done.send(status);
        }
    }
}

/// Why the thread should stop speaking what it is speaking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cut {
    /// Put the rest back, to be finished after the interruption.
    Resume,
    /// Abandon this message and move on to the next.
    Skip,
    /// Abandon everything.
    Clear,
}

/// What is queued, and what the thread should do about it.
struct Inner {
    /// High-priority messages, which jump the queue.
    urgent: VecDeque<Job>,
    /// A message that was interrupted and still owes its remainder.
    resume: Option<Job>,
    /// Everything else, in the order it arrived.
    normal: VecDeque<Job>,
    /// A pending instruction to the thread, consumed once acted on.
    cut: Option<Cut>,
    /// Whether to hold everything without discarding it.
    paused: bool,
    /// What is being spoken right now.
    speaking: Option<String>,
    /// Set when the node is going away.
    shutdown: bool,
}

impl Inner {
    /// The next message to speak.
    ///
    /// Urgent first, then anything interrupted, then the backlog. That order
    /// is what makes `high` mean "before the queue" rather than "instead of
    /// it": the interrupted message is still owed, and it comes back as soon
    /// as the urgent one is done rather than after everything queued behind.
    fn next(&mut self) -> Option<Job> {
        self.urgent
            .pop_front()
            .or_else(|| self.resume.take())
            .or_else(|| self.normal.pop_front())
    }

    /// How many messages are waiting, not counting the one being spoken.
    fn depth(&self) -> usize {
        self.urgent.len() + usize::from(self.resume.is_some()) + self.normal.len()
    }

    /// Every waiting message, in the order it will be spoken.
    fn pending(&self) -> Vec<String> {
        self.urgent
            .iter()
            .chain(self.resume.iter())
            .chain(self.normal.iter())
            .map(|j| j.msg_id.clone())
            .collect()
    }
}

/// What the queue looks like right now.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The message being spoken, if any.
    pub speaking: Option<String>,
    /// Messages waiting, in the order they will be spoken.
    pub pending: Vec<String>,
    /// Whether speaking is held.
    pub paused: bool,
}

/// The speaking thread and the queue feeding it.
#[derive(Clone)]
pub struct Speaker {
    inner: Arc<(Mutex<Inner>, Condvar)>,
    engine: Arc<dyn SpeechEngine>,
}

impl Speaker {
    /// Start the speaking thread.
    pub fn new(engine: Arc<dyn SpeechEngine>) -> Self {
        let inner = Arc::new((
            Mutex::new(Inner {
                urgent: VecDeque::new(),
                resume: None,
                normal: VecDeque::new(),
                cut: None,
                paused: false,
                speaking: None,
                shutdown: false,
            }),
            Condvar::new(),
        ));
        let speaker = Self {
            inner: Arc::clone(&inner),
            engine: Arc::clone(&engine),
        };
        let worker = speaker.clone();
        std::thread::spawn(move || worker.run());
        speaker
    }

    /// Queue a message.
    ///
    /// An urgent one interrupts whatever is playing; the interrupted message
    /// is put back and resumes at the chunk it was cut off in, so what the
    /// listener hears is a clean sentence restart rather than a fragment.
    pub fn submit(&self, job: Job, urgent: bool) {
        let (lock, cond) = &*self.inner;
        {
            let mut inner = lock.lock().expect("queue lock");
            if urgent {
                inner.urgent.push_back(job);
                // Only worth interrupting if something is actually playing.
                if inner.speaking.is_some() {
                    inner.cut = Some(Cut::Resume);
                }
            } else {
                inner.normal.push_back(job);
            }
            cond.notify_all();
        }
        // Outside the lock: stopping an engine can block on a child process,
        // and holding the queue while it does would stall every other caller.
        if urgent {
            self.engine.stop();
        }
    }

    /// Abandon the current message and carry on with the queue.
    pub fn skip(&self) {
        self.cut(Cut::Skip);
    }

    /// Drop one message, wherever it is.
    ///
    /// Returns whether it was found. A message already spoken is not an
    /// error, but the caller deserves to know nothing happened rather than
    /// be told a stale id was acted on.
    pub fn stop_message(&self, msg_id: &str) -> bool {
        let (lock, cond) = &*self.inner;
        let waiting = {
            let mut inner = lock.lock().expect("queue lock");
            if inner.speaking.as_deref() == Some(msg_id) {
                // Skip rather than clear: this asks for one message to stop,
                // not for the queue behind it to be thrown away.
                inner.cut = Some(Cut::Skip);
                cond.notify_all();
                drop(inner);
                self.engine.stop();
                return true;
            }
            if let Some(at) = inner.urgent.iter().position(|j| j.msg_id == msg_id) {
                inner.urgent.remove(at)
            } else if inner.resume.as_ref().is_some_and(|j| j.msg_id == msg_id) {
                inner.resume.take()
            } else if let Some(at) = inner.normal.iter().position(|j| j.msg_id == msg_id) {
                inner.normal.remove(at)
            } else {
                None
            }
        };
        match waiting {
            Some(job) => {
                job.finish(Status::Cancelled);
                true
            }
            None => false,
        }
    }

    /// Abandon everything, queue included.
    pub fn clear(&self) {
        let (lock, cond) = &*self.inner;
        let abandoned: Vec<Job> = {
            let mut inner = lock.lock().expect("queue lock");
            inner.cut = Some(Cut::Clear);
            let mut all: Vec<Job> = inner.urgent.drain(..).collect();
            all.extend(inner.resume.take());
            all.extend(inner.normal.drain(..));
            cond.notify_all();
            all
        };
        // Told individually rather than left to time out: each of these has a
        // caller that asked to be informed, and silence would read as a hang.
        for job in abandoned {
            job.finish(Status::Cancelled);
        }
        self.engine.stop();
    }

    /// Hold speech without discarding it.
    ///
    /// The message being spoken is put back whole at its current chunk, so
    /// resuming picks up where the sound stopped rather than skipping ahead.
    pub fn pause(&self) {
        let (lock, cond) = &*self.inner;
        {
            let mut inner = lock.lock().expect("queue lock");
            inner.paused = true;
            if inner.speaking.is_some() {
                inner.cut = Some(Cut::Resume);
            }
            cond.notify_all();
        }
        self.engine.stop();
    }

    /// Start speaking again after a pause.
    pub fn unpause(&self) {
        let (lock, cond) = &*self.inner;
        let mut inner = lock.lock().expect("queue lock");
        inner.paused = false;
        cond.notify_all();
    }

    /// Stop the thread, for a clean shutdown.
    pub fn shutdown(&self) {
        let (lock, cond) = &*self.inner;
        let mut inner = lock.lock().expect("queue lock");
        inner.shutdown = true;
        cond.notify_all();
    }

    /// How many messages are waiting behind the one being spoken.
    pub fn depth(&self) -> usize {
        self.inner.0.lock().expect("queue lock").depth()
    }

    /// What is playing and what is waiting.
    pub fn snapshot(&self) -> Snapshot {
        let inner = self.inner.0.lock().expect("queue lock");
        Snapshot {
            speaking: inner.speaking.clone(),
            pending: inner.pending(),
            paused: inner.paused,
        }
    }

    /// Ask the thread to stop what it is doing, and make the engine cooperate.
    fn cut(&self, cut: Cut) {
        let (lock, cond) = &*self.inner;
        {
            let mut inner = lock.lock().expect("queue lock");
            inner.cut = Some(cut);
            cond.notify_all();
        }
        self.engine.stop();
    }

    /// The speaking thread.
    fn run(&self) {
        let (lock, cond) = &*self.inner;
        loop {
            let job = {
                let mut inner = lock.lock().expect("queue lock");
                loop {
                    if inner.shutdown {
                        return;
                    }
                    if !inner.paused
                        && let Some(job) = inner.next()
                    {
                        // A cut left over from the message that just ended
                        // must not cancel the one about to start.
                        inner.cut = None;
                        inner.speaking = Some(job.msg_id.clone());
                        break job;
                    }
                    inner = cond.wait(inner).expect("queue lock");
                }
            };

            let outcome = self.speak_job(job);

            let mut inner = lock.lock().expect("queue lock");
            inner.speaking = None;
            // A message put back for later has no outcome yet.
            drop(inner);
            if let Some((job, status)) = outcome {
                job.finish(status);
            }
        }
    }

    /// Speak one message, returning it and its outcome unless it was put back.
    fn speak_job(&self, job: Job) -> Option<(Job, Status)> {
        let (lock, _) = &*self.inner;
        let Job {
            msg_id,
            chunks,
            voice,
            mut done,
        } = job;

        // Per-message voice, restored afterwards. Safe to do globally
        // because this thread is the only thing that speaks: messages are
        // serial by construction, so there is nothing to race with.
        let restore = voice.as_ref().and_then(|wanted| {
            let current = self.engine.current_voice();
            // Only worth restoring if the switch actually took. A device that
            // has never heard of the voice speaks in its own, as documented.
            self.engine.set_voice(wanted).ok().and(current)
        });

        let mut index = 0;
        let mut outcome = Status::Spoken;
        while index < chunks.len() {
            let spoke = self.engine.speak(&chunks[index]);

            // The cut is checked before the engine's own result, because
            // stopping an engine mid-sentence is how an interrupt works and
            // the error it returns is a consequence of that, not a fault.
            let cut = lock.lock().expect("queue lock").cut.take();
            match cut {
                Some(Cut::Resume) => {
                    // From this chunk, not the next: a sentence cut in half
                    // is restarted rather than dropped.
                    let mut inner = lock.lock().expect("queue lock");
                    inner.resume = Some(Job {
                        msg_id,
                        chunks: chunks[index..].to_vec(),
                        voice,
                        done: done.take(),
                    });
                    drop(inner);
                    if let Some(previous) = restore {
                        let _ = self.engine.set_voice(&previous);
                    }
                    return None;
                }
                Some(Cut::Skip) | Some(Cut::Clear) => {
                    outcome = Status::Cancelled;
                    break;
                }
                None => {
                    if spoke.is_err() {
                        outcome = Status::NoEngine;
                        break;
                    }
                    index += 1;
                }
            }
        }

        if let Some(previous) = restore {
            let _ = self.engine.set_voice(&previous);
        }

        Some((
            Job {
                msg_id,
                chunks,
                voice,
                done,
            },
            outcome,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use voicecast_engine::{EngineError, Tier, Voice};

    /// An engine that takes a measurable amount of time and can be cut off.
    ///
    /// Speaking has to actually block for any of this to be testable: an
    /// interrupt that arrives between chunks exercises none of the machinery
    /// that matters.
    struct FakeEngine {
        spoken: Mutex<Vec<String>>,
        stopping: Mutex<bool>,
        cv: Condvar,
    }

    impl FakeEngine {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                spoken: Mutex::new(Vec::new()),
                stopping: Mutex::new(false),
                cv: Condvar::new(),
            })
        }

        fn spoken(&self) -> Vec<String> {
            self.spoken.lock().expect("spoken").clone()
        }
    }

    impl SpeechEngine for FakeEngine {
        fn speak(&self, chunk: &str) -> Result<(), EngineError> {
            self.spoken.lock().expect("spoken").push(chunk.to_string());
            let stopping = self.stopping.lock().expect("stopping");
            let (mut stopping, _) = self
                .cv
                .wait_timeout(stopping, Duration::from_millis(120))
                .expect("wait");
            if *stopping {
                *stopping = false;
                return Err(EngineError::Unavailable("stopped".into()));
            }
            Ok(())
        }

        fn voices(&self) -> Vec<Voice> {
            Vec::new()
        }

        fn stop(&self) {
            *self.stopping.lock().expect("stopping") = true;
            self.cv.notify_all();
        }

        fn tier(&self) -> Tier {
            Tier::Full
        }
    }

    fn job(id: &str, chunks: &[&str]) -> (Job, tokio::sync::oneshot::Receiver<Status>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (
            Job {
                msg_id: id.to_string(),
                chunks: chunks.iter().map(|c| c.to_string()).collect(),
                voice: None,
                done: Some(tx),
            },
            rx,
        )
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(40)).await;
    }

    #[tokio::test]
    async fn urgent_interrupts_then_the_interrupted_message_resumes() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone());

        let (normal, normal_done) = job("m1", &["one", "two", "three"]);
        speaker.submit(normal, false);
        // Long enough to be *inside* the first chunk, which is the case the
        // resume logic exists for.
        settle().await;

        let (urgent, urgent_done) = job("m2", &["urgent"]);
        speaker.submit(urgent, true);

        let urgent_status = tokio::time::timeout(Duration::from_secs(5), urgent_done)
            .await
            .expect("urgent finished")
            .expect("urgent status");
        let normal_status = tokio::time::timeout(Duration::from_secs(5), normal_done)
            .await
            .expect("interrupted message finished")
            .expect("normal status");

        assert_eq!(urgent_status, Status::Spoken);
        assert_eq!(normal_status, Status::Spoken);
        // The cut-off chunk is spoken again from its start, so the listener
        // hears a whole sentence rather than the tail of one.
        assert_eq!(
            engine.spoken(),
            vec!["one", "urgent", "one", "two", "three"]
        );
        speaker.shutdown();
    }

    #[tokio::test]
    async fn skip_abandons_the_current_message_and_moves_on() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone());

        let (first, first_done) = job("m1", &["one", "two", "three"]);
        let (second, second_done) = job("m2", &["next"]);
        speaker.submit(first, false);
        speaker.submit(second, false);
        settle().await;

        speaker.skip();

        let first_status = tokio::time::timeout(Duration::from_secs(5), first_done)
            .await
            .expect("first finished")
            .expect("first status");
        let second_status = tokio::time::timeout(Duration::from_secs(5), second_done)
            .await
            .expect("second finished")
            .expect("second status");

        assert_eq!(first_status, Status::Cancelled);
        assert_eq!(second_status, Status::Spoken);
        // "two" and "three" are never reached, and nothing resumes.
        assert_eq!(engine.spoken(), vec!["one", "next"]);
        speaker.shutdown();
    }

    #[tokio::test]
    async fn clear_cancels_everything_including_what_is_waiting() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone());

        let (first, first_done) = job("m1", &["one", "two"]);
        let (second, second_done) = job("m2", &["never"]);
        speaker.submit(first, false);
        speaker.submit(second, false);
        settle().await;

        speaker.clear();

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), first_done)
                .await
                .expect("first finished")
                .expect("first status"),
            Status::Cancelled
        );
        // A queued message is told too, rather than left for its caller to
        // time out on.
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), second_done)
                .await
                .expect("second finished")
                .expect("second status"),
            Status::Cancelled
        );
        assert_eq!(engine.spoken(), vec!["one"]);
        speaker.shutdown();
    }

    #[tokio::test]
    async fn pause_holds_the_message_and_resume_finishes_it() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone());

        let (only, only_done) = job("m1", &["one", "two"]);
        speaker.submit(only, false);
        settle().await;

        speaker.pause();
        settle().await;
        assert!(speaker.snapshot().paused);
        // Held, not discarded: it is still owed.
        assert_eq!(speaker.depth(), 1);
        let while_paused = engine.spoken().len();

        speaker.unpause();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), only_done)
                .await
                .expect("finished")
                .expect("status"),
            Status::Spoken
        );
        assert!(engine.spoken().len() > while_paused);
        assert_eq!(engine.spoken().last().map(String::as_str), Some("two"));
        speaker.shutdown();
    }

    #[tokio::test]
    async fn the_queue_reports_what_is_waiting_in_playing_order() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone());

        speaker.submit(job("m1", &["a", "b"]).0, false);
        speaker.submit(job("m2", &["c"]).0, false);
        speaker.submit(job("m3", &["d"]).0, false);
        settle().await;

        let snap = speaker.snapshot();
        assert_eq!(snap.speaking.as_deref(), Some("m1"));
        assert_eq!(snap.pending, vec!["m2".to_string(), "m3".to_string()]);
        assert!(!snap.paused);
        speaker.shutdown();
    }
}
