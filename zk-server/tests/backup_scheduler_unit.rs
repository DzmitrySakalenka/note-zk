// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::{sync::Arc, time::Duration};

use chrono::{TimeZone, Utc};
use tokio::{
    sync::mpsc::{UnboundedReceiver, error::TryRecvError},
    time::{Instant, advance, timeout},
};
use tokio_util::sync::CancellationToken;
use zk_server::scheduler::backup::spawn_backup_scheduler_tick_runner;

const CRON_EVERY_FIVE_MINUTES: &str = "*/5 * * * *";
const FIVE_MINUTES: Duration = Duration::from_secs(5 * 60);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn tick_runner_fires_once_per_virtual_cron_period() {
    let shutdown = CancellationToken::new();
    let virtual_started_at = Instant::now();
    let utc_started_at = Utc
        .with_ymd_and_hms(2026, 5, 3, 0, 0, 0)
        .single()
        .expect("test UTC timestamp should be valid");
    let (tick_sender, mut tick_receiver) = tokio::sync::mpsc::unbounded_channel();
    let tick_counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let handle = spawn_backup_scheduler_tick_runner(
        CRON_EVERY_FIVE_MINUTES.to_string(),
        shutdown.clone(),
        move || {
            let elapsed = Instant::now().duration_since(virtual_started_at);
            utc_started_at
                + chrono::Duration::from_std(elapsed)
                    .expect("test virtual time should fit in chrono duration")
        },
        {
            let tick_counter = Arc::clone(&tick_counter);
            move || {
                let tick_sender = tick_sender.clone();
                let tick_counter = Arc::clone(&tick_counter);
                async move {
                    let tick = tick_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    tick_sender
                        .send(tick)
                        .expect("test receiver should remain open");
                }
            }
        },
    );

    tokio::task::yield_now().await;

    for expected_tick in 1..=7 {
        advance(FIVE_MINUTES).await;

        assert_eq!(
            receive_tick(&mut tick_receiver).await,
            expected_tick,
            "scheduler should fire once after each cron period"
        );
        assert!(matches!(tick_receiver.try_recv(), Err(TryRecvError::Empty)));

        tokio::task::yield_now().await;
    }

    shutdown.cancel();
    timeout(Duration::from_secs(1), handle.join())
        .await
        .expect("scheduler should join after cancellation");
}

async fn receive_tick(tick_receiver: &mut UnboundedReceiver<usize>) -> usize {
    timeout(Duration::from_secs(1), tick_receiver.recv())
        .await
        .expect("scheduler should tick within bounded virtual time")
        .expect("tick sender should remain open")
}
