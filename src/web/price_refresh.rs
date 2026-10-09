use std::{future::Future, time::Duration};
use tokio::{sync::watch, task::JoinHandle};

use super::{
    error::WebError,
    state::{AppState, PriceRefresh},
};

pub(super) fn start(
    state: AppState,
    interval: Duration,
    stop: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(run(interval, stop, move || {
        let state = state.clone();
        async move { state.refresh_prices().await }
    }))
}

async fn run<F, R>(interval: Duration, mut stop: watch::Receiver<bool>, mut refresh: F)
where
    F: FnMut() -> R,
    R: Future<Output = Result<PriceRefresh, WebError>>,
{
    if interval.is_zero() {
        tracing::info!("automatic price refresh disabled");
        return;
    }
    tracing::info!(
        interval_seconds = interval.as_secs(),
        "automatic price refresh enabled"
    );
    loop {
        if *stop.borrow() || stop.has_changed().is_err() {
            break;
        }
        tracing::debug!("starting scheduled price refresh");
        log_result(refresh().await);
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            _ = tokio::time::sleep(interval) => {},
        }
    }
    tracing::info!("automatic price refresh stopped");
}

fn log_result(result: Result<PriceRefresh, WebError>) {
    match result {
        Ok(PriceRefresh::Updated(summary)) => tracing::info!(
            updated = summary.updated,
            unsupported = summary.unsupported,
            "scheduled price refresh saved"
        ),
        Ok(PriceRefresh::AlreadyRunning) => {
            tracing::debug!("scheduled price refresh skipped; another refresh is running")
        }
        Err(error) => {
            tracing::warn!(error = %error.message, status = error.status.as_u16(), "scheduled price refresh failed; will retry after the configured interval")
        }
    }
}

#[cfg(test)]
mod tests;
