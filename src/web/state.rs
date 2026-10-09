use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use super::error::WebError;
use crate::{
    model::StoreData,
    price_sync::{PriceBatch, SyncSummary, fetch_prices, merge_price_batch},
    store::Store,
};

#[derive(Clone)]
pub(super) struct AppState {
    store: Arc<Mutex<Store>>,
    refreshing: Arc<AtomicBool>,
    pub port: u16,
}

struct RefreshGuard(Arc<AtomicBool>);

pub(super) enum PriceRefresh {
    Updated(SyncSummary),
    AlreadyRunning,
}

impl RefreshGuard {
    fn acquire(refreshing: Arc<AtomicBool>) -> Option<Self> {
        refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(Self(refreshing))
    }
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl AppState {
    pub fn new(store: Store, port: u16) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            refreshing: Arc::new(AtomicBool::new(false)),
            port,
        }
    }

    pub async fn snapshot(&self) -> Result<StoreData, WebError> {
        let state = self.clone();
        blocking(move || Ok(state.store.lock().map_err(lock_error)?.data.clone())).await
    }

    pub async fn edit<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Store) -> anyhow::Result<T> + Send + 'static,
    ) -> Result<T, WebError> {
        let state = self.clone();
        blocking(move || state.commit(operation)).await
    }

    fn commit<T>(
        &self,
        operation: impl FnOnce(&mut Store) -> anyhow::Result<T>,
    ) -> Result<T, WebError> {
        let mut current = self.store.lock().map_err(lock_error)?;
        let mut candidate = current.clone();
        let result = operation(&mut candidate).map_err(WebError::invalid)?;
        candidate.save()?;
        *current = candidate;
        Ok(result)
    }

    pub fn is_refreshing(&self) -> bool {
        self.refreshing.load(Ordering::Acquire)
    }

    pub async fn refresh_prices(&self) -> Result<PriceRefresh, WebError> {
        self.refresh_with(fetch_prices).await
    }

    async fn refresh_with(
        &self,
        fetch: impl FnOnce(&StoreData) -> anyhow::Result<PriceBatch> + Send + 'static,
    ) -> Result<PriceRefresh, WebError> {
        let Some(guard) = RefreshGuard::acquire(self.refreshing.clone()) else {
            return Ok(PriceRefresh::AlreadyRunning);
        };
        let state = self.clone();
        blocking(move || {
            let _guard = guard;
            state.fetch_and_commit(fetch)
        })
        .await
        .map(PriceRefresh::Updated)
    }

    fn fetch_and_commit(
        &self,
        fetch: impl FnOnce(&StoreData) -> anyhow::Result<PriceBatch>,
    ) -> Result<SyncSummary, WebError> {
        let data = self.store.lock().map_err(lock_error)?.data.clone();
        let batch = fetch(&data).map_err(provider_error)?;
        self.commit(move |store| Ok(merge_price_batch(&mut store.data, &data, batch)))
    }
}

fn provider_error(error: anyhow::Error) -> WebError {
    tracing::warn!(%error, "price refresh failed; existing prices retained");
    WebError {
        status: axum::http::StatusCode::BAD_GATEWAY,
        message: "The price provider could not be reached. Existing prices have been retained. Try again shortly.".into(),
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> WebError {
    anyhow::anyhow!("datastore lock was poisoned").into()
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, WebError> + Send + 'static,
) -> Result<T, WebError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(anyhow::Error::from)?
}

#[cfg(test)]
mod tests;
