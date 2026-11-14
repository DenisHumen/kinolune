// SPDX-License-Identifier: MPL-2.0
//! A running chat for one view (TWI-012): source → render → batches.
//!
//! [`ChatSession`] owns a thread with a single-threaded Tokio runtime. It
//! pulls events from a [`ChatSource`], renders messages with the channel's
//! [`ChannelAssets`], downloads the images they need through [`ChatImages`]
//! on a few worker threads (each image once per key) and hands everything
//! to the consumer in batches every `flush_interval` (research 06 §3.3:
//! render batching every 100–200 ms). A busy chat of 100 messages per
//! second thus costs the UI five to ten updates per second, not a hundred.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use pa_twitch::SentChatMessage;
use tokio::sync::{Notify, mpsc};

use crate::assets::{ChannelAssets, ImagePrefs};
use crate::filters::ChatFilters;
use crate::images::{ChatImages, ImageRequest, image_requests};
use crate::model::{ChatEvent, ChatMessage, SourceStatus};
use crate::render::RenderedMessage;
use crate::source::{ChatError, ChatSource};

/// How long [`ChatSession::send`] waits for Helix.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// Image keys remembered as requested; beyond this the set restarts (a
/// key asked again is served from the disk cache).
const MAX_REQUESTED_KEYS: usize = 8192;

/// Settings of a [`ChatSession`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// Image density, theme and animation.
    pub prefs: ImagePrefs,
    /// How often a batch is handed over (default 150 ms).
    pub flush_interval: Duration,
    /// Image download threads (default 2).
    pub image_workers: usize,
    /// Most messages kept in one batch; older ones of the same batch are
    /// dropped (the UI keeps fewer than this anyway). Default 1000.
    pub max_batch_messages: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            prefs: ImagePrefs::default(),
            flush_interval: Duration::from_millis(150),
            image_workers: 2,
            max_batch_messages: 1000,
        }
    }
}

/// A chat line ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    /// The normalised message.
    pub message: Box<ChatMessage>,
    /// Its badges and fragments.
    pub rendered: RenderedMessage,
    /// It mentions the signed-in user or a highlight keyword (TWI-024).
    pub mention: bool,
}

/// An image is on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReady {
    /// [`ImageRequest::key`].
    pub key: String,
    /// Local file.
    pub path: PathBuf,
    /// The file is an animation (decode it once and share it).
    pub animated: bool,
}

/// What a session hands to the consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// A rendered message.
    Line(ChatLine),
    /// Any other source event (moderation, room modes, notices, status).
    Event(ChatEvent),
    /// An image finished downloading.
    Image(ImageReady),
    /// An image could not be fetched; the UI keeps the text fallback.
    ImageFailed {
        /// [`ImageRequest::key`].
        key: String,
    },
}

/// A request from [`ChatSession::send`] to the session thread.
struct SendRequest {
    broadcaster_id: String,
    text: String,
    reply_to: Option<String>,
    reply: std_mpsc::Sender<Result<SentChatMessage, ChatError>>,
}

type SharedFilters = Arc<Mutex<Arc<ChatFilters>>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A chat being pumped into a consumer; stops on drop.
#[derive(Debug)]
pub struct ChatSession {
    stop: Arc<Notify>,
    filters: SharedFilters,
    sends: mpsc::UnboundedSender<SendRequest>,
}

impl std::fmt::Debug for SendRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendRequest").finish_non_exhaustive()
    }
}

impl ChatSession {
    /// Starts the session thread. `make_source` runs inside its Tokio
    /// runtime (sources spawn tasks there). `sink` gets the batches and
    /// returns `false` to stop. `images: None` skips downloads.
    pub fn start<F, S>(
        make_source: F,
        assets: Arc<ChannelAssets>,
        images: Option<ChatImages>,
        config: SessionConfig,
        sink: S,
    ) -> std::io::Result<Self>
    where
        F: FnOnce() -> Result<Box<dyn ChatSource>, ChatError> + Send + 'static,
        S: FnMut(Vec<SessionEvent>) -> bool + Send + 'static,
    {
        Self::start_filtered(
            make_source,
            assets,
            images,
            config,
            Arc::new(ChatFilters::default()),
            sink,
        )
    }

    /// [`ChatSession::start`] with viewer filters applied from the first
    /// message (TWI-024).
    pub fn start_filtered<F, S>(
        make_source: F,
        assets: Arc<ChannelAssets>,
        images: Option<ChatImages>,
        config: SessionConfig,
        filters: Arc<ChatFilters>,
        sink: S,
    ) -> std::io::Result<Self>
    where
        F: FnOnce() -> Result<Box<dyn ChatSource>, ChatError> + Send + 'static,
        S: FnMut(Vec<SessionEvent>) -> bool + Send + 'static,
    {
        let stop = Arc::new(Notify::new());
        let stop_rx = Arc::clone(&stop);
        let filters: SharedFilters = Arc::new(Mutex::new(filters));
        let shared = Arc::clone(&filters);
        let (sends, send_rx) = mpsc::unbounded_channel();
        thread::Builder::new()
            .name("twitch-chat".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(r) => r,
                    Err(e) => {
                        let mut sink = sink;
                        sink(vec![stopped(&e.to_string())]);
                        return;
                    }
                };
                runtime.block_on(pump(
                    make_source,
                    Pumped {
                        assets,
                        images,
                        config,
                        filters: shared,
                        sends: send_rx,
                    },
                    sink,
                    stop_rx,
                ));
            })?;
        Ok(Self {
            stop,
            filters,
            sends,
        })
    }

    /// Replaces the viewer filters; messages from now on use them (lines
    /// already handed over stay as they were).
    pub fn set_filters(&self, filters: Arc<ChatFilters>) {
        *lock(&self.filters) = filters;
    }

    /// Sends `text` to the channel `broadcaster_id` through the session's
    /// source (Helix `POST /chat/messages` for the signed-in source) and
    /// waits for the answer. Blocks: do not call it on the session's
    /// runtime. A message Twitch dropped (slow mode, sub-only, duplicate…)
    /// is `Ok` with `is_sent == false` and the reason.
    pub fn send(
        &self,
        broadcaster_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<SentChatMessage, ChatError> {
        let (reply, answer) = std_mpsc::channel();
        self.sends
            .send(SendRequest {
                broadcaster_id: broadcaster_id.to_owned(),
                text: text.to_owned(),
                reply_to: reply_to.map(str::to_owned),
                reply,
            })
            .map_err(|_| ChatError::Closed)?;
        match answer.recv_timeout(SEND_TIMEOUT) {
            Ok(result) => result,
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                Err(ChatError::Api("no answer from Twitch".into()))
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => Err(ChatError::Closed),
        }
    }

    /// Stops the session; the sink gets no more batches after the current
    /// one.
    pub fn stop(&self) {
        self.stop.notify_one();
    }
}

impl Drop for ChatSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn stopped(reason: &str) -> SessionEvent {
    SessionEvent::Event(ChatEvent::Status(SourceStatus::Stopped {
        reason: reason.to_owned(),
    }))
}

/// What the session thread owns besides the source and the sink.
struct Pumped {
    assets: Arc<ChannelAssets>,
    images: Option<ChatImages>,
    config: SessionConfig,
    filters: SharedFilters,
    sends: mpsc::UnboundedReceiver<SendRequest>,
}

async fn pump<F, S>(make_source: F, pumped: Pumped, mut sink: S, stop: Arc<Notify>)
where
    F: FnOnce() -> Result<Box<dyn ChatSource>, ChatError>,
    S: FnMut(Vec<SessionEvent>) -> bool,
{
    let Pumped {
        assets,
        images,
        config,
        filters,
        mut sends,
    } = pumped;
    let source: Arc<dyn ChatSource> = match make_source() {
        Ok(s) => Arc::from(s),
        Err(e) => {
            sink(vec![stopped(&e.to_string())]);
            return;
        }
    };
    let (done_tx, done_rx) = std_mpsc::channel::<SessionEvent>();
    let downloads = images.map(|images| Downloader::start(images, config.image_workers, done_tx));
    let mut batch = Batch::new(config.max_batch_messages);
    let mut requested: HashSet<String> = HashSet::new();
    let mut tick = tokio::time::interval(config.flush_interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut ended = false;
    while !ended {
        tokio::select! {
            biased;
            () = stop.notified() => break,
            Some(request) = sends.recv() => {
                let source = Arc::clone(&source);
                // Its own task: the feed keeps flowing while Helix answers.
                tokio::spawn(async move {
                    let result = source
                        .send_message(
                            &request.broadcaster_id,
                            &request.text,
                            request.reply_to.as_deref(),
                        )
                        .await;
                    let _ = request.reply.send(result);
                });
            }
            _ = tick.tick() => {
                while let Ok(e) = done_rx.try_recv() {
                    batch.push(e);
                }
                if !batch.is_empty() && !sink(batch.take()) {
                    break;
                }
            }
            event = source.next_event() => match event {
                None => ended = true,
                Some(ChatEvent::Message(mut message)) => {
                    let current = Arc::clone(&lock(&filters));
                    let verdict = current.apply(&mut message);
                    if verdict.hidden.is_some() {
                        continue;
                    }
                    let rendered = assets.render(&message);
                    if let Some(d) = &downloads {
                        if requested.len() > MAX_REQUESTED_KEYS {
                            requested.clear();
                        }
                        for r in image_requests([&rendered], &config.prefs) {
                            if requested.insert(r.key.clone()) {
                                d.request(r);
                            }
                        }
                    }
                    batch.push(SessionEvent::Line(ChatLine {
                        message,
                        rendered,
                        mention: verdict.mention,
                    }));
                }
                Some(other) => batch.push(SessionEvent::Event(other)),
            },
        }
    }
    source.close();
    if ended {
        while let Ok(e) = done_rx.try_recv() {
            batch.push(e);
        }
        if !batch.is_empty() {
            sink(batch.take());
        }
    }
    drop(downloads);
}

/// Events of one flush with a cap on lines.
struct Batch {
    events: Vec<SessionEvent>,
    lines: usize,
    max_lines: usize,
}

impl Batch {
    fn new(max_lines: usize) -> Self {
        Self {
            events: Vec::new(),
            lines: 0,
            max_lines: max_lines.max(1),
        }
    }

    fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    fn push(&mut self, e: SessionEvent) {
        if matches!(e, SessionEvent::Line(_)) {
            self.lines += 1;
        }
        self.events.push(e);
        if self.lines > self.max_lines {
            // Drop the oldest line of this batch: nobody could read it.
            if let Some(i) = self
                .events
                .iter()
                .position(|e| matches!(e, SessionEvent::Line(_)))
            {
                self.events.remove(i);
                self.lines -= 1;
            }
        }
    }

    fn take(&mut self) -> Vec<SessionEvent> {
        self.lines = 0;
        std::mem::take(&mut self.events)
    }
}

/// Image download threads fed by a queue.
struct Downloader {
    queue: Option<std_mpsc::Sender<ImageRequest>>,
}

impl Downloader {
    fn start(images: ChatImages, workers: usize, done: std_mpsc::Sender<SessionEvent>) -> Self {
        let (tx, rx) = std_mpsc::channel::<ImageRequest>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..workers.max(1) {
            let rx = Arc::clone(&rx);
            let images = images.clone();
            let done = done.clone();
            let _ = thread::Builder::new()
                .name(format!("twitch-chat-img-{i}"))
                .spawn(move || {
                    loop {
                        let next = rx.lock().map(|r| r.recv());
                        let Ok(Ok(request)) = next else { return };
                        let event = match images.get(&request) {
                            Ok(path) => {
                                let animated = request.animated.unwrap_or_else(|| is_gif(&path));
                                SessionEvent::Image(ImageReady {
                                    key: request.key,
                                    path,
                                    animated,
                                })
                            }
                            Err(_) => SessionEvent::ImageFailed { key: request.key },
                        };
                        if done.send(event).is_err() {
                            return;
                        }
                    }
                });
        }
        Self { queue: Some(tx) }
    }

    fn request(&self, r: ImageRequest) {
        if let Some(q) = &self.queue {
            let _ = q.send(r);
        }
    }
}

impl Drop for Downloader {
    fn drop(&mut self) {
        // Closing the queue ends the workers after their current download.
        self.queue.take();
    }
}

fn is_gif(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 12];
    std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .map(|n| pa_imagecache::sniff_image(&head[..n]) == Some("gif"))
        .unwrap_or(false)
}
