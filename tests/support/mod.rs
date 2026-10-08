use std::sync::atomic::{AtomicU64, Ordering};
use tuifolio::store::Store;

pub fn temp_store() -> Store {
    Store::open(Some(temp_store_path())).unwrap()
}

pub fn temp_store_path() -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "tuifolio-test-{}-{timestamp}-{counter}.json",
        std::process::id()
    ))
}
