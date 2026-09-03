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

use voicecast_engine::{EngineError, SpeechEngine};
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
    pub done: Option<tokio::sync::oneshot::Sender<Ended>>,
}

/// How a message ended, and why when the status alone does not say.
///
/// The status used to travel on its own, so every engine failure arrived as
/// `NoEngine` with the reason dropped on the floor. A receiver with a working
/// Piper and no audio session then told the sender to install a speech engine
/// that was already there, which is the opposite of the fix (#86).
#[derive(Debug, Clone)]
pub struct Ended {
    /// What happened.
    pub status: Status,
    /// Why, when there is more to say than the status.
    pub detail: Option<String>,
}

impl Ended {
    /// An outcome that speaks for itself.
    pub fn plain(status: Status) -> Self {
        Self {
            status,
            detail: None,
        }
    }
}

impl Job {
    /// Tell a waiting caller how this ended, if anyone is still listening.
    fn finish(self, ended: Ended) {
        if let Some(done) = self.done {
            // Ignored deliberately: the caller may have stopped waiting.
            let _ = done.send(ended);
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
    /// How many words that message has, since it is no longer in any queue.
    ///
    /// Held separately because `next` takes the job out of the deques to
    /// speak it, so a word count that only walks those sees nothing at all
    /// while a device is mid-sentence.
    speaking_words: usize,
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

/// Told how every message ended, whoever was or was not waiting for it.
///
/// The `done` channel only exists when a caller asked to wait, and most do
/// not — so it cannot be what keeps a record. This fires either way.
pub type OnFinish = Arc<dyn Fn(&str, Ended) + Send + Sync>;

/// The speaking thread and the queue feeding it.
#[derive(Clone)]
pub struct Speaker {
    inner: Arc<(Mutex<Inner>, Condvar)>,
    engine: Arc<dyn SpeechEngine>,
    on_finish: OnFinish,
}

impl Speaker {
    /// Start the speaking thread.
    pub fn new(engine: Arc<dyn SpeechEngine>, on_finish: OnFinish) -> Self {
        let inner = Arc::new((
            Mutex::new(Inner {
                urgent: VecDeque::new(),
                resume: None,
                normal: VecDeque::new(),
                cut: None,
                paused: false,
                speaking: None,
                speaking_words: 0,
                shutdown: false,
            }),
            Condvar::new(),
        ));
        let speaker = Self {
            inner: Arc::clone(&inner),
            engine: Arc::clone(&engine),
            on_finish,
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
                (self.on_finish)(&job.msg_id, Ended::plain(Status::Cancelled));
                job.finish(Ended::plain(Status::Cancelled));
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
            (self.on_finish)(&job.msg_id, Ended::plain(Status::Cancelled));
            job.finish(Ended::plain(Status::Cancelled));
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

    /// How many words are still to be spoken, across everything queued.
    ///
    /// Used to work out how long a caller asking to wait should be prepared
    /// to wait: its own message is only part of the answer when something is
    /// already talking.
    ///
    /// The message being spoken is counted whole, since how much of it is
    /// left is not tracked — an over-estimate, which is the safe direction
    /// for a bound on waiting. It has to be counted from a field rather than
    /// from the queues, because the thread takes it out of them to speak it.
    pub fn pending_words(&self) -> usize {
        let inner = self.inner.0.lock().expect("queue lock");
        inner.speaking_words
            + inner
                .urgent
                .iter()
                .chain(inner.resume.iter())
                .chain(inner.normal.iter())
                .map(|j| words_in(&j.chunks))
                .sum::<usize>()
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
                        inner.speaking_words = words_in(&job.chunks);
                        break job;
                    }
                    inner = cond.wait(inner).expect("queue lock");
                }
            };

            let outcome = self.speak_job(job);

            let mut inner = lock.lock().expect("queue lock");
            inner.speaking = None;
            // Whatever became of it, it is no longer owed by this slot: a
            // message put back for later is counted again through `resume`.
            inner.speaking_words = 0;
            // A message put back for later has no outcome yet.
            drop(inner);
            if let Some((job, ended)) = outcome {
                (self.on_finish)(&job.msg_id, ended.clone());
                job.finish(ended);
            }
        }
    }

    /// Speak one message, returning it and its outcome unless it was put back.
    fn speak_job(&self, job: Job) -> Option<(Job, Ended)> {
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
        let mut outcome = Ended::plain(Status::Spoken);
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
                    outcome = Ended::plain(Status::Cancelled);
                    break;
                }
                None => {
                    if let Err(e) = spoke {
                        // "There is no engine" and "the engine ran and
                        // failed" want opposite responses — install one
                        // versus diagnose the one you have — and both used to
                        // arrive as NoEngine with the reason discarded (#86).
                        outcome = Ended {
                            status: match e {
                                EngineError::Unavailable(_) => Status::NoEngine,
                                _ => Status::Unreachable,
                            },
                            detail: Some(e.to_string()),
                        };
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

/// Words across a set of chunks, counted the way a speech engine consumes
/// them: whitespace-separated, punctuation and all.
pub fn words_in(chunks: &[String]) -> usize {
    chunks.iter().map(|c| c.split_whitespace().count()).sum()
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
        /// Returned instead of speaking, for the failure paths.
        fails_with: Option<EngineError>,
    }

    impl FakeEngine {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                spoken: Mutex::new(Vec::new()),
                stopping: Mutex::new(false),
                cv: Condvar::new(),
                fails_with: None,
            })
        }

        /// An engine that is present and refuses, which is a different thing
        /// to an engine that is not there (#86).
        fn failing(error: EngineError) -> Arc<Self> {
            Arc::new(Self {
                spoken: Mutex::new(Vec::new()),
                stopping: Mutex::new(false),
                cv: Condvar::new(),
                fails_with: Some(error),
            })
        }

        fn spoken(&self) -> Vec<String> {
            self.spoken.lock().expect("spoken").clone()
        }
    }

    impl SpeechEngine for FakeEngine {
        fn speak(&self, chunk: &str) -> Result<(), EngineError> {
            self.spoken.lock().expect("spoken").push(chunk.to_string());
            if let Some(e) = &self.fails_with {
                return Err(e.clone());
            }
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

    fn job(id: &str, chunks: &[&str]) -> (Job, tokio::sync::oneshot::Receiver<Ended>) {
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

    #[tokio::test]
    async fn an_engine_that_ran_and_failed_is_not_reported_as_a_missing_one() {
        // #86. The two need opposite responses — install an engine, versus
        // diagnose the one you have — and both used to arrive as `NoEngine`
        // with the reason thrown away, so a receiver whose audio device was
        // gone sent the sender to download a voice model.
        let engine = FakeEngine::failing(EngineError::Failed {
            command: "paplay".into(),
            code: "exit code 1".into(),
            detail: Some("connection refused".into()),
        });
        let speaker = Speaker::new(engine, Arc::new(|_, _| {}));
        let (job, done) = job("m1", &["hello"]);
        speaker.submit(job, false);

        let ended = tokio::time::timeout(Duration::from_secs(5), done)
            .await
            .expect("finished in time")
            .expect("job finished");
        assert_eq!(
            ended.status,
            Status::Unreachable,
            "an engine that ran is not a missing engine"
        );
        let why = ended.detail.expect("the reason travels with it");
        assert!(why.contains("paplay"), "names the command: {why}");
        assert!(why.contains("connection refused"), "and why: {why}");
        assert!(why.contains("exit code 1"), "and how it ended: {why}");
    }

    #[tokio::test]
    async fn a_genuinely_missing_engine_still_says_so() {
        let engine =
            FakeEngine::failing(EngineError::Unavailable("no voice model installed".into()));
        let speaker = Speaker::new(engine, Arc::new(|_, _| {}));
        let (job, done) = job("m1", &["hello"]);
        speaker.submit(job, false);

        let ended = tokio::time::timeout(Duration::from_secs(5), done)
            .await
            .expect("finished in time")
            .expect("job finished");
        assert_eq!(ended.status, Status::NoEngine);
        assert!(ended.detail.is_some_and(|d| d.contains("voice model")));
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(40)).await;
    }

    /// Words counted the way the estimate needs them.
    #[test]
    fn words_are_counted_across_chunks() {
        assert_eq!(words_in(&[]), 0);
        assert_eq!(words_in(&["one two".into(), "three".into()]), 3);
        // Punctuation rides along with the word it belongs to, and repeated
        // or surrounding whitespace does not invent extra ones.
        assert_eq!(words_in(&["  Dr. Smith,   again.  ".into()]), 3);
    }

    /// A message being spoken counts towards what a later caller waits for.
    ///
    /// It is not in any queue while it plays — the thread takes it out to
    /// speak it — so a count that only walked the queues reported nothing at
    /// all mid-sentence. A short message sent while a long one was playing
    /// would then be given the floor of thirty seconds to wait through
    /// however many minutes were still coming out of the speaker, which is
    /// the very failure this change exists to remove.
    #[tokio::test]
    async fn what_is_being_spoken_counts_towards_the_wait() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone(), Arc::new(|_, _| {}));

        let lines: Vec<String> = (0..40)
            .map(|i| format!("sentence number {i} here"))
            .collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let (long, _done) = job("long", &refs);
        speaker.submit(long, false);
        settle().await;

        // Playing, and therefore in none of the queues.
        assert_eq!(speaker.snapshot().speaking.as_deref(), Some("long"));
        assert_eq!(speaker.depth(), 0, "it is out of the queues while it plays");
        assert!(
            speaker.pending_words() >= 100,
            "a message mid-flight still has to be waited through; got {}",
            speaker.pending_words()
        );

        speaker.clear();
        settle().await;
        assert_eq!(speaker.pending_words(), 0, "a cleared queue owes nothing");
    }

    /// What is still to be said includes what is waiting, not just what is
    /// playing — a caller waits through the whole queue in front of it.
    #[tokio::test]
    async fn pending_words_covers_everything_still_to_be_spoken() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone(), Arc::new(|_, _| {}));
        assert_eq!(speaker.pending_words(), 0);

        let (first, _first_done) = job("m1", &["one two three", "four five"]);
        speaker.submit(first, false);
        let (second, _second_done) = job("m2", &["six seven"]);
        speaker.submit(second, false);

        // Counted before the thread has drained anything, so both are still
        // in hand. Over-counting as speaking proceeds is the safe direction:
        // this bounds how long a caller waits, and waiting ends when speech
        // does.
        assert!(
            speaker.pending_words() > 0,
            "queued messages should count towards the wait"
        );

        speaker.clear();
        settle().await;
        assert_eq!(speaker.pending_words(), 0, "a cleared queue owes nothing");
    }

    #[tokio::test]
    async fn urgent_interrupts_then_the_interrupted_message_resumes() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone(), Arc::new(|_, _| {}));

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

        assert_eq!(urgent_status.status, Status::Spoken);
        assert_eq!(normal_status.status, Status::Spoken);
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
        let speaker = Speaker::new(engine.clone(), Arc::new(|_, _| {}));

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

        assert_eq!(first_status.status, Status::Cancelled);
        assert_eq!(second_status.status, Status::Spoken);
        // "two" and "three" are never reached, and nothing resumes.
        assert_eq!(engine.spoken(), vec!["one", "next"]);
        speaker.shutdown();
    }

    #[tokio::test]
    async fn clear_cancels_everything_including_what_is_waiting() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone(), Arc::new(|_, _| {}));

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
                .expect("first status")
                .status,
            Status::Cancelled
        );
        // A queued message is told too, rather than left for its caller to
        // time out on.
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), second_done)
                .await
                .expect("second finished")
                .expect("second status")
                .status,
            Status::Cancelled
        );
        assert_eq!(engine.spoken(), vec!["one"]);
        speaker.shutdown();
    }

    #[tokio::test]
    async fn pause_holds_the_message_and_resume_finishes_it() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone(), Arc::new(|_, _| {}));

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
                .expect("status")
                .status,
            Status::Spoken
        );
        assert!(engine.spoken().len() > while_paused);
        assert_eq!(engine.spoken().last().map(String::as_str), Some("two"));
        speaker.shutdown();
    }

    #[tokio::test]
    async fn the_queue_reports_what_is_waiting_in_playing_order() {
        let engine = FakeEngine::new();
        let speaker = Speaker::new(engine.clone(), Arc::new(|_, _| {}));

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
