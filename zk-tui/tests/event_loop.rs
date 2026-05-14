#![allow(unused_crate_dependencies)]

use std::{
    collections::VecDeque,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, TryRecvError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use zk_tui::event::{
    AppEvent, AppEventSender, BackgroundResult, Clock, EventLoop, SystemClock, TerminalEventSource,
    run_worker_for_test,
};

const EVENT_CHANNEL_CAPACITY: usize = 64;

#[test]
fn background_result_round_trips_opaque_payload() {
    let result = BackgroundResult::new(Box::new(String::from("scan complete")));

    let payload = result.into_payload();
    let Ok(message) = payload.downcast::<String>() else {
        panic!("background payload should downcast to string");
    };

    assert_eq!(*message, "scan complete");
}

#[test]
fn app_event_sender_delivers_background_results() {
    let (tx, rx): (AppEventSender, Receiver<AppEvent>) = sync_channel(1);

    tx.send(AppEvent::Background(BackgroundResult::new(Box::new(7_u32))))
        .expect("background event should send");

    let AppEvent::Background(result) = rx.recv().expect("background event should arrive") else {
        panic!("expected background event");
    };

    let Ok(payload) = result.into_payload().downcast::<u32>() else {
        panic!("background payload should downcast to u32");
    };

    assert_eq!(*payload, 7);
}

#[test]
fn event_loop_public_api_is_reachable() {
    fn assert_start_signature(_start: fn(Duration) -> EventLoop) {}
    fn assert_recv_signature(
        _recv: fn(&EventLoop) -> Result<AppEvent, std::sync::mpsc::RecvError>,
    ) {
    }
    fn assert_sender_signature(_sender: fn(&EventLoop) -> AppEventSender) {}

    assert_start_signature(EventLoop::start);
    assert_recv_signature(EventLoop::recv);
    assert_sender_signature(EventLoop::sender);
}

#[derive(Clone)]
struct ManualClock {
    now: Arc<Mutex<Instant>>,
}

impl ManualClock {
    fn new(now: Instant) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    fn advance(&self, duration: Duration) {
        let mut now = self.now.lock().expect("manual clock should lock");
        *now += duration;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().expect("manual clock should lock")
    }
}

enum PollStep {
    Ready(Event),
    Timeout,
}

struct FakeInput {
    clock: ManualClock,
    shutdown: Arc<AtomicBool>,
    poll_timeouts: Arc<Mutex<Vec<Duration>>>,
    steps: VecDeque<PollStep>,
    pending_event: Option<Event>,
}

impl FakeInput {
    fn new(
        clock: ManualClock,
        shutdown: Arc<AtomicBool>,
        poll_timeouts: Arc<Mutex<Vec<Duration>>>,
        steps: Vec<PollStep>,
    ) -> Self {
        Self {
            clock,
            shutdown,
            poll_timeouts,
            steps: VecDeque::from(steps),
            pending_event: None,
        }
    }
}

impl TerminalEventSource for FakeInput {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        self.poll_timeouts
            .lock()
            .expect("poll timeout recorder should lock")
            .push(timeout);

        match self.steps.pop_front() {
            Some(PollStep::Ready(event)) => {
                self.pending_event = Some(event);
                Ok(true)
            }
            Some(PollStep::Timeout) => {
                self.clock.advance(timeout);
                Ok(false)
            }
            None => {
                self.shutdown.store(true, Ordering::Release);
                Ok(false)
            }
        }
    }

    fn read(&mut self) -> io::Result<Event> {
        self.pending_event
            .take()
            .ok_or_else(|| io::Error::other("fake input read without pending event"))
    }
}

struct ExitRecorderInput {
    dropped: Arc<AtomicBool>,
}

impl TerminalEventSource for ExitRecorderInput {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        let sleep_for = timeout.min(Duration::from_millis(1));
        if !sleep_for.is_zero() {
            thread::sleep(sleep_for);
        }

        Ok(false)
    }

    fn read(&mut self) -> io::Result<Event> {
        Err(io::Error::other("exit recorder has no events"))
    }
}

impl Drop for ExitRecorderInput {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
    }
}

#[test]
fn worker_emits_ticks_on_mocked_cadence() {
    let tick = Duration::from_millis(250);
    let clock = ManualClock::new(Instant::now());
    let shutdown = Arc::new(AtomicBool::new(false));
    let poll_timeouts = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = sync_channel(EVENT_CHANNEL_CAPACITY);
    let mut input = FakeInput::new(
        clock.clone(),
        Arc::clone(&shutdown),
        Arc::clone(&poll_timeouts),
        vec![PollStep::Timeout, PollStep::Timeout],
    );

    run_worker_for_test(&mut input, &clock, &tx, &shutdown, tick);

    assert!(shutdown.load(Ordering::Acquire));
    assert!(matches!(
        rx.recv().expect("first tick should arrive"),
        AppEvent::Tick
    ));
    assert!(matches!(
        rx.recv().expect("second tick should arrive"),
        AppEvent::Tick
    ));

    let recorded = poll_timeouts
        .lock()
        .expect("poll timeout recorder should lock");
    assert_eq!(recorded[0], tick);
    assert_eq!(recorded[1], tick);
}

#[test]
fn worker_classifies_key_and_resize_events() {
    let tick = Duration::from_millis(250);
    let clock = ManualClock::new(Instant::now());
    let shutdown = Arc::new(AtomicBool::new(false));
    let poll_timeouts = Arc::new(Mutex::new(Vec::new()));
    let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    let (tx, rx) = sync_channel(EVENT_CHANNEL_CAPACITY);
    let mut input = FakeInput::new(
        clock.clone(),
        Arc::clone(&shutdown),
        poll_timeouts,
        vec![
            PollStep::Ready(Event::Key(key)),
            PollStep::Ready(Event::Resize(80, 24)),
        ],
    );

    run_worker_for_test(&mut input, &clock, &tx, &shutdown, tick);

    let AppEvent::Key(received_key) = rx.recv().expect("key event should arrive") else {
        panic!("expected key event");
    };
    assert_eq!(received_key, key);

    let AppEvent::Resize(columns, rows) = rx.recv().expect("resize event should arrive") else {
        panic!("expected resize event");
    };
    assert_eq!((columns, rows), (80, 24));
}

#[test]
fn worker_drops_events_when_channel_is_full() {
    let tick = Duration::from_millis(250);
    let clock = ManualClock::new(Instant::now());
    let shutdown = Arc::new(AtomicBool::new(false));
    let poll_timeouts = Arc::new(Mutex::new(Vec::new()));
    let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    let (tx, rx) = sync_channel(1);
    assert!(tx.try_send(AppEvent::Tick).is_ok());

    let mut input = FakeInput::new(
        clock.clone(),
        Arc::clone(&shutdown),
        poll_timeouts,
        vec![PollStep::Ready(Event::Key(key))],
    );

    run_worker_for_test(&mut input, &clock, &tx, &shutdown, tick);

    assert!(matches!(
        rx.recv().expect("preloaded tick should arrive"),
        AppEvent::Tick
    ));
    assert!(matches!(
        rx.try_recv(),
        Err(TryRecvError::Empty | TryRecvError::Disconnected)
    ));
}

#[test]
fn dropping_event_loop_flips_shutdown_and_joins_worker() {
    let dropped = Arc::new(AtomicBool::new(false));
    let event_loop = EventLoop::start_with_source(
        ExitRecorderInput {
            dropped: Arc::clone(&dropped),
        },
        SystemClock,
        Duration::from_millis(1),
    );
    let shutdown = event_loop.shutdown_flag();

    drop(event_loop);

    assert!(shutdown.load(Ordering::Acquire));
    assert!(dropped.load(Ordering::Acquire));
}
