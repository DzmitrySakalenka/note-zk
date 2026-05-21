#![allow(unused_crate_dependencies)]

use std::{
    sync::mpsc::{Receiver, RecvTimeoutError, sync_channel},
    time::Duration,
};

use zk_tui::{
    event::{AppEvent, BackgroundResult},
    jobs::{JobContext, JobKind, JobResult, JobRunner, JobSpec, MockJob},
};

const EVENT_CHANNEL_CAPACITY: usize = 64;

#[test]
fn immediate_cancelled_mock_job_returns_cancelled_within_one_hundred_ms() {
    let (tx, rx) = sync_channel(EVENT_CHANNEL_CAPACITY);
    let runner = JobRunner::start(tx, test_context());
    let handle = runner.enqueue(JobSpec::Mock(MockJob::new(
        "slow",
        Duration::from_millis(500),
    )));

    handle.cancel();

    let result = recv_job_result(&rx, Duration::from_millis(100));
    assert!(handle.is_cancelled());
    assert!(matches!(
        result,
        JobResult::Cancelled {
            kind: JobKind::Mock,
        }
    ));
}

fn recv_job_result(rx: &Receiver<AppEvent>, timeout: Duration) -> JobResult {
    let event = rx
        .recv_timeout(timeout)
        .unwrap_or_else(|error| match error {
            RecvTimeoutError::Timeout => panic!("timed out waiting for job result"),
            RecvTimeoutError::Disconnected => panic!("job event channel disconnected"),
        });

    let AppEvent::Background(result) = event else {
        panic!("expected background job result");
    };

    downcast_job_result(result)
}

fn downcast_job_result(result: BackgroundResult) -> JobResult {
    let Ok(result) = result.into_payload().downcast::<JobResult>() else {
        panic!("background payload should downcast to JobResult");
    };

    *result
}

fn test_context() -> JobContext {
    let config = zk_app::EffectiveConfig::default();
    JobContext::new(config.clone(), config.vault_path.clone())
}
