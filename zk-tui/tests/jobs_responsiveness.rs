#![allow(unused_crate_dependencies)]

use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{RecvTimeoutError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::Rect,
    widgets::{Paragraph, Widget},
};
use zk_tui::{
    app::{App, Screen, ScreenAction, ScreenCtx},
    event::{AppEvent, AppEventSender, Clock, TerminalEventSource, run_worker_for_test},
    jobs::{JobContext, JobResult, JobRunner, JobSpec, MockJob},
};

const WIDTH: u16 = 64;
const HEIGHT: u16 = 6;
const EVENT_CHANNEL_CAPACITY: usize = 64;
const TICK: Duration = Duration::from_millis(50);
const JOB_DELAY: Duration = Duration::from_millis(500);
const SIMULATED_RUN: Duration = Duration::from_millis(600);

#[test]
fn render_loop_keeps_ticking_while_mock_job_is_in_flight() {
    let (tx, rx) = sync_channel(EVENT_CHANNEL_CAPACITY);
    let tick_worker = start_tick_worker(tx.clone(), TICK, SIMULATED_RUN);
    let runner = JobRunner::start(tx, test_context());
    let state = Arc::new(Mutex::new(ProbeState::in_flight()));
    let ctx = ScreenCtx::for_test().expect("screen context should initialize");
    let mut app = App::with_screens(vec![Box::new(ProbeScreen::new(Arc::clone(&state)))], ctx);
    let mut terminal =
        Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal should initialize");
    let started = Instant::now();

    runner.enqueue(JobSpec::Mock(MockJob::new("slow", JOB_DELAY)));
    draw(&mut terminal, &mut app);

    let mut background_results = 0;
    let result_elapsed = loop {
        let event = recv_event(&rx, Duration::from_secs(1));
        let result_elapsed = mock_result_elapsed(&event, started);
        assert!(app.handle_event(event));
        draw(&mut terminal, &mut app);

        if let Some(elapsed) = result_elapsed {
            background_results += 1;
            break elapsed;
        }
    };

    tick_worker
        .join()
        .expect("tick worker should finish cleanly");
    background_results += drain_background_results(&rx, Duration::from_millis(100));

    let state = state.lock().expect("probe state should lock");
    assert!(
        state.ticks_while_in_flight >= 8,
        "expected at least 8 ticks while job ran, got {}",
        state.ticks_while_in_flight
    );
    assert!(state.rendered_in_flight);
    assert_eq!(state.mock_results, 1);
    assert_eq!(background_results, 1);
    assert!(result_elapsed >= JOB_DELAY);
}

struct ProbeScreen {
    state: Arc<Mutex<ProbeState>>,
}

impl ProbeScreen {
    fn new(state: Arc<Mutex<ProbeState>>) -> Self {
        Self { state }
    }
}

impl Screen for ProbeScreen {
    fn title(&self) -> &'static str {
        "Job Probe"
    }

    fn handle_event(&mut self, event: &AppEvent, _ctx: &mut ScreenCtx) -> ScreenAction {
        let mut state = self.state.lock().expect("probe state should lock");
        match event {
            AppEvent::Tick if state.in_flight => {
                state.ticks_while_in_flight += 1;
            }
            AppEvent::Background(result) => {
                if matches!(result.downcast_ref::<JobResult>(), Some(JobResult::Mock(_))) {
                    state.in_flight = false;
                    state.mock_results += 1;
                }
            }
            AppEvent::Key(_) | AppEvent::Tick | AppEvent::Resize(_, _) => {}
        }

        ScreenAction::Stay
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, _ctx: &ScreenCtx) {
        let mut state = self.state.lock().expect("probe state should lock");
        let text = if state.in_flight {
            state.rendered_in_flight = true;
            let spinner = ["-", "\\", "|", "/"][state.ticks_while_in_flight % 4];
            format!("in-flight {spinner}")
        } else {
            "complete".to_string()
        };

        Paragraph::new(text).render(area, frame.buffer_mut());
    }
}

struct ProbeState {
    in_flight: bool,
    rendered_in_flight: bool,
    ticks_while_in_flight: usize,
    mock_results: usize,
}

impl ProbeState {
    const fn in_flight() -> Self {
        Self {
            in_flight: true,
            rendered_in_flight: false,
            ticks_while_in_flight: 0,
            mock_results: 0,
        }
    }
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

    fn elapsed_since(&self, start: Instant) -> Duration {
        self.now()
            .checked_duration_since(start)
            .expect("manual clock should not move backwards")
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().expect("manual clock should lock")
    }
}

struct SimulatedTimeoutInput {
    clock: ManualClock,
    shutdown: Arc<AtomicBool>,
    started: Instant,
    run_for: Duration,
}

impl TerminalEventSource for SimulatedTimeoutInput {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        let elapsed = self.clock.elapsed_since(self.started);
        if elapsed >= self.run_for {
            self.shutdown.store(true, Ordering::Release);
            return Ok(false);
        }

        self.clock
            .advance(timeout.min(self.run_for.saturating_sub(elapsed)));
        Ok(false)
    }

    fn read(&mut self) -> io::Result<crossterm::event::Event> {
        Err(io::Error::other("simulated timeout input has no events"))
    }
}

fn start_tick_worker(
    tx: AppEventSender,
    tick: Duration,
    run_for: Duration,
) -> thread::JoinHandle<()> {
    let started = Instant::now();
    let clock = ManualClock::new(started);
    let shutdown = Arc::new(AtomicBool::new(false));
    let mut input = SimulatedTimeoutInput {
        clock: clock.clone(),
        shutdown: Arc::clone(&shutdown),
        started,
        run_for,
    };

    thread::spawn(move || {
        run_worker_for_test(&mut input, &clock, &tx, &shutdown, tick);
    })
}

fn recv_event(rx: &std::sync::mpsc::Receiver<AppEvent>, timeout: Duration) -> AppEvent {
    rx.recv_timeout(timeout)
        .unwrap_or_else(|error| match error {
            RecvTimeoutError::Timeout => panic!("timed out waiting for TUI event"),
            RecvTimeoutError::Disconnected => panic!("TUI event channel disconnected"),
        })
}

fn mock_result_elapsed(event: &AppEvent, started: Instant) -> Option<Duration> {
    let AppEvent::Background(result) = event else {
        return None;
    };
    let Some(JobResult::Mock(mock)) = result.downcast_ref::<JobResult>() else {
        return None;
    };
    assert_eq!(mock.label, "slow");

    Some(started.elapsed())
}

fn drain_background_results(rx: &std::sync::mpsc::Receiver<AppEvent>, timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    let mut count = 0;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(AppEvent::Background(_)) => count += 1,
            Ok(AppEvent::Key(_) | AppEvent::Tick | AppEvent::Resize(_, _)) => {}
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return count,
        }
    }

    count
}

fn test_context() -> JobContext {
    let config = zk_app::EffectiveConfig::default();
    JobContext::new(config.clone(), config.vault_path.clone())
}

fn draw(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    terminal
        .draw(|frame| app.render(frame))
        .expect("app should render to test backend");
}
