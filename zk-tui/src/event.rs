//! Terminal input polling and application event dispatch.

use std::{
    any::Any,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvError, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crossterm::event::{self, Event, KeyEvent};

const EVENT_CHANNEL_CAPACITY: usize = 64;

/// Event sender shared by background jobs and the terminal input worker.
pub type AppEventSender = SyncSender<AppEvent>;

/// Unified event stream consumed by the synchronous render loop.
pub enum AppEvent {
    /// Terminal key input from crossterm.
    Key(KeyEvent),
    /// Fixed-cadence render tick.
    Tick,
    /// Terminal resize with column and row counts.
    Resize(u16, u16),
    /// Result from a background job.
    Background(BackgroundResult),
}

/// Opaque background job payload placeholder.
pub struct BackgroundResult {
    payload: Box<dyn Any + Send>,
}

impl BackgroundResult {
    /// Wraps an opaque background job payload.
    pub fn new(payload: Box<dyn Any + Send>) -> Self {
        Self { payload }
    }

    /// Returns the wrapped opaque payload.
    pub fn into_payload(self) -> Box<dyn Any + Send> {
        self.payload
    }
}

/// Worker-backed event dispatcher for terminal input and render ticks.
pub struct EventLoop {
    tx: AppEventSender,
    rx: Receiver<AppEvent>,
    handle: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl EventLoop {
    /// Starts a crossterm input worker with the provided tick cadence.
    pub fn start(tick: Duration) -> Self {
        Self::start_with_source(CrosstermInput, SystemClock, tick)
    }

    /// Receives the next application event.
    ///
    /// # Errors
    ///
    /// Returns an error when every event sender has been dropped.
    pub fn recv(&self) -> Result<AppEvent, RecvError> {
        self.rx.recv()
    }

    /// Returns a sender that can enqueue application events.
    pub fn sender(&self) -> AppEventSender {
        self.tx.clone()
    }

    #[doc(hidden)]
    pub fn start_with_source<I, C>(input: I, clock: C, tick: Duration) -> Self
    where
        I: TerminalEventSource + Send + 'static,
        C: Clock + Send + 'static,
    {
        let (tx, rx) = sync_channel(EVENT_CHANNEL_CAPACITY);
        let worker_tx = tx.clone();
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            let mut input = input;
            run_worker(&mut input, &clock, &worker_tx, &worker_shutdown, tick);
        });

        Self {
            tx,
            rx,
            handle: Some(handle),
            shutdown,
        }
    }

    #[doc(hidden)]
    pub fn shutdown_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);

        if let Some(handle) = self.handle.take() {
            if handle.join().is_err() {
                tracing::warn!("event loop worker panicked during shutdown");
            }
        }
    }
}

#[doc(hidden)]
pub trait TerminalEventSource {
    /// Polls for a terminal event for at most the provided timeout.
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;

    /// Reads a terminal event after polling has reported one is ready.
    fn read(&mut self) -> io::Result<Event>;
}

struct CrosstermInput;

impl TerminalEventSource for CrosstermInput {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        event::poll(timeout)
    }

    fn read(&mut self) -> io::Result<Event> {
        event::read()
    }
}

#[doc(hidden)]
pub trait Clock {
    /// Returns the current clock instant.
    fn now(&self) -> Instant;
}

#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

fn run_worker<I, C>(
    input: &mut I,
    clock: &C,
    tx: &AppEventSender,
    shutdown: &Arc<AtomicBool>,
    tick: Duration,
) where
    I: TerminalEventSource,
    C: Clock,
{
    let mut next_tick = clock.now() + tick;

    while !shutdown.load(Ordering::Acquire) {
        let timeout = next_tick.saturating_duration_since(clock.now());
        let poll_result = input.poll(timeout);

        if shutdown.load(Ordering::Acquire) {
            break;
        }

        match poll_result {
            Ok(true) => {
                if !read_and_dispatch(input, tx) {
                    break;
                }
            }
            Ok(false) => {
                if !send_app_event(tx, AppEvent::Tick) {
                    break;
                }
                next_tick = clock.now() + tick;
            }
            Err(error) => {
                tracing::warn!(%error, "failed to poll terminal input");
            }
        }
    }
}

#[doc(hidden)]
pub fn run_worker_for_test<I, C>(
    input: &mut I,
    clock: &C,
    tx: &AppEventSender,
    shutdown: &Arc<AtomicBool>,
    tick: Duration,
) where
    I: TerminalEventSource,
    C: Clock,
{
    run_worker(input, clock, tx, shutdown, tick);
}

fn read_and_dispatch<I>(input: &mut I, tx: &AppEventSender) -> bool
where
    I: TerminalEventSource,
{
    match input.read() {
        Ok(event) => classify_event(&event).is_none_or(|event| send_app_event(tx, event)),
        Err(error) => {
            tracing::warn!(%error, "failed to read terminal input");
            true
        }
    }
}

fn classify_event(event: &Event) -> Option<AppEvent> {
    match event {
        Event::Key(key) => Some(AppEvent::Key(*key)),
        Event::Resize(columns, rows) => Some(AppEvent::Resize(*columns, *rows)),
        _ => None,
    }
}

fn send_app_event(tx: &AppEventSender, event: AppEvent) -> bool {
    match tx.try_send(event) {
        Ok(()) => true,
        Err(TrySendError::Full(_event)) => {
            tracing::warn!("TUI event channel is full; dropping event");
            true
        }
        Err(TrySendError::Disconnected(_event)) => {
            tracing::warn!("TUI event channel disconnected; stopping event loop");
            false
        }
    }
}
