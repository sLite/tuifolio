use super::{PriceRefresh, WebError, run};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle, time::advance};

const PERIOD: Duration = Duration::from_secs(super::super::DEFAULT_PRICE_REFRESH_SECONDS);

fn spawn_job(stop: watch::Receiver<bool>, calls: Arc<AtomicUsize>) -> JoinHandle<()> {
    tokio::spawn(run(PERIOD, stop, move || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Ok(PriceRefresh::AlreadyRunning) }
    }))
}

#[tokio::test(start_paused = true)]
async fn refreshes_immediately_and_then_after_each_configured_interval() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (stop, receiver) = watch::channel(false);
    let job = spawn_job(receiver, calls.clone());
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    advance(PERIOD - Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    stop.send_replace(true);
    job.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn zero_interval_disables_startup_and_periodic_refreshes() {
    let calls = AtomicUsize::new(0);
    let (_stop, receiver) = watch::channel(false);
    run(Duration::ZERO, receiver, || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Ok(PriceRefresh::AlreadyRunning) }
    })
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn provider_errors_do_not_stop_future_scheduled_attempts() {
    let calls = Arc::new(AtomicUsize::new(0));
    let attempts = calls.clone();
    let (stop, receiver) = watch::channel(false);
    let job = tokio::spawn(run(PERIOD, receiver, move || {
        attempts.fetch_add(1, Ordering::SeqCst);
        async { Err(WebError::invalid("test failure")) }
    }));
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    advance(PERIOD).await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    stop.send_replace(true);
    job.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn shutdown_finishes_an_active_refresh_without_starting_another() {
    let calls = Arc::new(AtomicUsize::new(0));
    let attempts = calls.clone();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let wait = gate.clone();
    let (stop, receiver) = watch::channel(false);
    let job = tokio::spawn(run(PERIOD, receiver, move || {
        attempts.fetch_add(1, Ordering::SeqCst);
        let wait = wait.clone();
        async move {
            wait.acquire().await.unwrap().forget();
            Ok(PriceRefresh::AlreadyRunning)
        }
    }));
    tokio::task::yield_now().await;
    stop.send_replace(true);
    assert!(!job.is_finished());
    gate.add_permits(1);
    job.await.unwrap();
    advance(PERIOD * 2).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn slow_refreshes_do_not_overlap_or_trigger_catch_up_bursts() {
    let calls = Arc::new(AtomicUsize::new(0));
    let attempts = calls.clone();
    let (stop, receiver) = watch::channel(false);
    let job = tokio::spawn(run(PERIOD, receiver, move || {
        attempts.fetch_add(1, Ordering::SeqCst);
        async {
            tokio::time::sleep(PERIOD * 2).await;
            Ok(PriceRefresh::AlreadyRunning)
        }
    }));
    tokio::task::yield_now().await;
    advance(PERIOD * 2).await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    advance(PERIOD).await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    stop.send_replace(true);
    advance(PERIOD * 2).await;
    job.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn closed_or_already_stopped_channels_do_not_start_refreshes() {
    let calls = AtomicUsize::new(0);
    for stopping in [false, true] {
        let (stop, receiver) = watch::channel(stopping);
        drop(stop);
        run(PERIOD, receiver, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok(PriceRefresh::AlreadyRunning) }
        })
        .await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
