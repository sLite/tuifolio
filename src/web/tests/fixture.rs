use std::path::PathBuf;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::Response,
};
use rust_decimal_macros::dec;
use tower::ServiceExt;

use super::super::{forms::TransactionForm, router, state::AppState};
use crate::{
    model::{AssetKind, Id, StoreData},
    price_sync::add_manual_price,
    store::Store,
    transactions::add_manual_transaction,
};

pub(super) struct TestApp {
    pub directory: tempfile::TempDir,
    pub path: PathBuf,
    pub state: AppState,
    pub app: Router,
}

impl TestApp {
    pub fn new(seeded: bool) -> Self {
        Self::open(if seeded { seed } else { |_| {} })
    }

    pub fn with_assets() -> Self {
        Self::open(configure_assets)
    }

    fn open(initialize: fn(&mut Store)) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data/store.json");
        let mut store = Store::open(Some(path.clone())).unwrap();
        initialize(&mut store);
        store.save().unwrap();
        let state = AppState::new(store, 3000);
        Self {
            directory,
            path,
            app: router(state.clone()),
            state,
        }
    }

    pub async fn get(&self, path: &str) -> Response {
        self.request("GET", path, "", None).await
    }

    pub async fn post(&self, path: &str, body: &str) -> Response {
        self.request("POST", path, body, None).await
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: &str,
        htmx: Option<&str>,
    ) -> Response {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("Host", "127.0.0.1:3000")
            .header("Origin", "http://127.0.0.1:3000")
            .header("Content-Type", "application/x-www-form-urlencoded");
        if let Some(htmx) = htmx {
            request = request.header("HX-Request", htmx);
        }
        self.app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    pub fn persisted(&self) -> StoreData {
        serde_json::from_reader(std::fs::File::open(&self.path).unwrap()).unwrap()
    }

    pub async fn asset_id(&self, symbol: &str) -> Id {
        self.state
            .snapshot()
            .await
            .unwrap()
            .assets
            .iter()
            .find(|a| a.symbol == symbol)
            .unwrap()
            .id
    }

    pub async fn buy_body(&self) -> String {
        buy_body(&self.state.snapshot().await.unwrap())
    }

    pub async fn buy_form(&self) -> TransactionForm {
        buy_form(&self.state.snapshot().await.unwrap())
    }
}

fn configure_assets(store: &mut Store) {
    store.asset_id_with_metadata(
        "BTC",
        "Bitcoin",
        AssetKind::Crypto,
        Some("BTC-USD".into()),
        Some("CRYPTO:BTCUSD".into()),
        None,
    );
    store.asset_id("EUR", "Euro", AssetKind::Fiat);
    store.asset_id("USD", "US dollar", AssetKind::Fiat);
    store.portfolio_id("Main");
}

fn seed(store: &mut Store) {
    configure_assets(store);
    let mut buy = buy_form(&store.data);
    buy.base_amount = "0.1".into();
    buy.quote_asset_id = asset_id(&store.data, "USD").to_string();
    buy.fee_asset_id = asset_id(&store.data, "EUR").to_string();
    buy.fee_amount = "2".into();
    buy.notes = "<script>alert('ledger')</script>".into();
    add_manual_transaction(store, buy.input().unwrap()).unwrap();
    let mut deposit = buy_form(&store.data);
    deposit.kind = "Deposit".into();
    deposit.base_asset_id = asset_id(&store.data, "EUR").to_string();
    deposit.base_amount = "5000".into();
    deposit.quote_asset_id.clear();
    deposit.quote_amount.clear();
    add_manual_transaction(store, deposit.input().unwrap()).unwrap();
    add_manual_price(store, "BTC", dec!(30000), "EUR").unwrap();
    add_manual_price(store, "USD", dec!(0.9), "EUR").unwrap();
}

pub(super) fn buy_form(data: &StoreData) -> TransactionForm {
    TransactionForm {
        portfolio_id: portfolio_id(data).to_string(),
        timestamp: "2026-10-08T12:30".into(),
        kind: "Buy".into(),
        base_asset_id: asset_id(data, "BTC").to_string(),
        base_amount: "0.123456789123456789".into(),
        quote_asset_id: asset_id(data, "EUR").to_string(),
        quote_amount: "123.123456789".into(),
        quote_ledger_effect: "Ignore".into(),
        ..TransactionForm::default()
    }
}

fn buy_body(data: &StoreData) -> String {
    format!(
        "portfolio_id={}&timestamp=2026-10-08T12%3A30&kind=Buy&base_asset_id={}&base_amount=0.123456789123456789&quote_asset_id={}&quote_amount=123.123456789&quote_ledger_effect=Ignore",
        portfolio_id(data),
        asset_id(data, "BTC"),
        asset_id(data, "EUR")
    )
}

fn portfolio_id(data: &StoreData) -> Id {
    data.portfolios
        .iter()
        .find(|portfolio| portfolio.name == "Main")
        .unwrap()
        .id
}

fn asset_id(data: &StoreData, symbol: &str) -> Id {
    data.assets
        .iter()
        .find(|asset| asset.symbol == symbol)
        .unwrap()
        .id
}

pub(super) fn repeat_transactions(mut data: StoreData, count: usize) -> StoreData {
    let transaction = data.transactions[0].clone();
    for _ in 0..count {
        let mut next = transaction.clone();
        next.id = data.allocate_id();
        data.transactions.push(next);
    }
    data
}

pub(super) async fn html(response: Response, status: StatusCode) -> String {
    assert_eq!(response.status(), status);
    String::from_utf8(
        to_bytes(response.into_body(), 5_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}
