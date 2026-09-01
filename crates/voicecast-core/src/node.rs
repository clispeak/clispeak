//! The node: accepts local requests and speaks them.
//!
//! At M3 this is deliberately the smallest thing that works — one machine,
//! no network, no roster. Transport and membership arrive at M5. What is
//! already real is the shape: a long-lived process owning the engine and a
//! queue, with the CLI as a thin client that hands over text and exits.

use std::sync::Arc;

use anyhow::{Context, Result};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, ToNsName, tokio::Stream, traits::tokio::Listener,
};
use voicecast_engine::SpeechEngine;

use crate::Identity;
use voicecast_proto::{Priority, Request, Response, Status};
use voicecast_text::chunk;

use crate::ipc::{read_frame, socket_name, write_frame};

/// A speech request waiting its turn.
struct Job {
    /// Identifies the message for control commands.
    msg_id: String,
    /// Sentence-sized pieces, in order.
    chunks: Vec<String>,
}

/// The running node.
pub struct Node {
    engine: Arc<dyn SpeechEngine>,
    /// This device's keypair. Reported by `status`, and handed to the
    /// transport at M5.
    identity: Identity,
    /// Jobs are handed to a dedicated blocking thread; speech is serial by
    /// nature, so a channel is the whole scheduler at this stage.
    tx: tokio::sync::mpsc::UnboundedSender<Job>,
    queued: Arc<std::sync::atomic::AtomicUsize>,
}

impl Node {
    /// Start a node with the given engine and identity.
    pub fn new(engine: Arc<dyn SpeechEngine>, identity: Identity) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let queued = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let worker_engine = Arc::clone(&engine);
        let worker_queued = Arc::clone(&queued);
        // Speaking blocks, so it gets its own thread rather than starving the
        // runtime that is accepting connections.
        std::thread::spawn(move || {
            while let Some(job) = rx.blocking_recv() {
                for c in &job.chunks {
                    if let Err(e) = worker_engine.speak(c) {
                        eprintln!("[{}] speech failed: {e}", job.msg_id);
                        break;
                    }
                }
                worker_queued.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        });

        Self {
            engine,
            identity,
            tx,
            queued,
        }
    }

    /// Listen for CLI connections until cancelled.
    pub async fn serve(&self) -> Result<()> {
        let name = socket_name()
            .to_ns_name::<GenericNamespaced>()
            .context("building socket name")?;
        let listener = ListenerOptions::new()
            .name(name)
            .create_tokio()
            .context("another node may already be running")?;

        eprintln!("voicecast node listening on {}", socket_name());

        loop {
            let stream = listener.accept().await.context("accepting connection")?;
            if let Err(e) = self.handle(stream).await {
                eprintln!("connection error: {e}");
            }
        }
    }

    /// Serve one CLI connection.
    ///
    /// Handled inline rather than spawned: requests are tiny and speech is
    /// serial anyway, so concurrency here would buy nothing but interleaved
    /// error output.
    async fn handle(&self, mut stream: Stream) -> Result<()> {
        let request: Request = read_frame(&mut stream).await?;

        let response = match request {
            Request::Speak { text, priority } => self.accept(text, priority),
            Request::Stop => {
                self.engine.stop();
                Response::Finished {
                    status: Status::Cancelled,
                }
            }
            Request::Status => Response::Status {
                device_id: self.identity.id().to_string(),
                key_store: self.identity.location().to_string(),
                engine: self
                    .engine
                    .voices()
                    .first()
                    .map_or("unknown", |v| &v.name)
                    .to_string(),
                fallback: self.engine.tier() == voicecast_engine::Tier::Fallback,
                queued: self.queued.load(std::sync::atomic::Ordering::SeqCst),
            },
        };

        write_frame(&mut stream, &response).await
    }

    /// Queue text for speaking and acknowledge immediately.
    fn accept(&self, text: String, priority: Priority) -> Response {
        let chunks = chunk(&text);
        if chunks.is_empty() {
            return Response::Error {
                message: "nothing to say".into(),
            };
        }

        let msg_id = new_msg_id();

        // High priority interrupts. Resume-after-interrupt is M8; for now the
        // interrupted message is simply dropped, which is honest but not yet
        // what `docs/cli.md` promises.
        if priority == Priority::High {
            self.engine.stop();
        }

        self.queued
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match self.tx.send(Job {
            msg_id: msg_id.clone(),
            chunks,
        }) {
            Ok(()) => Response::Accepted { msg_id },
            Err(_) => Response::Error {
                message: "speech worker has stopped".into(),
            },
        }
    }
}

/// A short, unique-enough message id.
fn new_msg_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("m_{:x}", nanos as u64 & 0xffff_ffff)
}
