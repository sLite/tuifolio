use std::sync::atomic::{AtomicU64, Ordering};
use tuifolio::store::Store;
use tuifolio::{
    assets::{AssetInput, save_asset},
    model::{AssetKind, Id},
    portfolios::{PortfolioInput, create_portfolio as save_portfolio},
};

pub fn create_asset(store: &mut Store, symbol: &str, name: &str, kind: AssetKind) -> Id {
    save_asset(
        store,
        None,
        AssetInput {
            symbol: symbol.into(),
            name: name.into(),
            kind,
            yahoo_symbol: None,
            tradingview_symbol: None,
            valuation_currency: None,
        },
    )
    .unwrap()
}

pub fn create_portfolio(store: &mut Store, name: &str) -> Id {
    save_portfolio(store, PortfolioInput { name: name.into() }).unwrap()
}

pub fn duplicate_asset(store: &mut Store, original: Id, name: &str, kind: AssetKind) -> Id {
    let mut asset = store
        .data
        .assets
        .iter()
        .find(|asset| asset.id == original)
        .unwrap()
        .clone();
    asset.id = store.data.allocate_id();
    asset.name = name.into();
    asset.kind = kind;
    let id = asset.id;
    store.data.assets.push(asset);
    id
}

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
