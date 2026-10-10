use super::{AppState, PriceRefresh};
use crate::{
    price_sync::{PriceBatch, SyncSummary},
    store::Store,
};
use axum::http::StatusCode;
use std::{sync::mpsc, time::Duration};

struct Fixture {
    directory: tempfile::TempDir,
    state: AppState,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(Some(directory.path().join("data/store.json"))).unwrap();
        store.save().unwrap();
        Self {
            directory,
            state: AppState::new(store, 3000),
        }
    }
}

#[tokio::test]
async fn invalid_stored_usd_configuration_returns_actionable_errors_without_changing_data() {
    use crate::{
        assets::{AssetInput, save_asset},
        model::AssetKind,
    };
    let fixture = Fixture::new();
    fixture
        .state
        .edit(|store| {
            for symbol in ["EUR", "USD"] {
                save_asset(
                    store,
                    None,
                    AssetInput {
                        symbol: symbol.into(),
                        name: symbol.into(),
                        kind: AssetKind::Fiat,
                        yahoo_symbol: None,
                        tradingview_symbol: None,
                        valuation_currency: None,
                    },
                )?;
            }
            // Bypass current validation to exercise previously accepted stored data.
            store
                .data
                .assets
                .iter_mut()
                .find(|asset| asset.symbol == "USD")
                .unwrap()
                .kind = AssetKind::Crypto;
            crate::price_sync::add_manual_price(store, "EUR", rust_decimal::Decimal::ONE, "USD")
        })
        .await
        .unwrap();
    let before = serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap();
    let path = fixture.directory.path().join("data/store.json");
    let bytes = std::fs::read(&path).unwrap();
    for _ in 0..2 {
        let error = fixture.state.refresh_prices().await.err().unwrap();
        assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(error.message.contains("USD must use the Cash asset type"));
        assert!(!fixture.state.is_refreshing());
        assert_eq!(
            serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
            before
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    // Repair the asset, then verify the next refresh job can use the slot.
    fixture
        .state
        .edit(|store| {
            let usd = store.asset_by_symbol("USD").unwrap().clone();
            save_asset(
                store,
                Some(usd.id),
                AssetInput {
                    symbol: usd.symbol,
                    name: usd.name,
                    kind: AssetKind::Fiat,
                    yahoo_symbol: None,
                    tradingview_symbol: None,
                    valuation_currency: None,
                },
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .state
            .refresh_with(|data| {
                assert!(
                    data.assets
                        .iter()
                        .any(|asset| asset.symbol == "USD" && asset.kind == AssetKind::Fiat)
                );
                Ok(empty_batch())
            })
            .await
            .unwrap(),
        PriceRefresh::Updated(_)
    ));
}

#[tokio::test]
async fn post_replacement_durability_error_publishes_saved_state_and_reports_truthfully() {
    let fixture = Fixture::new();
    let error = fixture
        .state
        .commit_with_save(
            |store| {
                crate::portfolios::create_portfolio(
                    store,
                    crate::portfolios::PortfolioInput {
                        name: "Committed".into(),
                    },
                )?;
                Ok(())
            },
            |store| {
                store.save()?;
                Err(crate::store::SaveDurabilityError {
                    source: std::io::Error::other("injected directory-sync failure"),
                }
                .into())
            },
        )
        .unwrap_err();
    assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(error.message.contains("store was saved"));
    assert!(!error.message.contains("not saved"));
    let saved: crate::model::StoreData = serde_json::from_slice(
        &std::fs::read(fixture.directory.path().join("data/store.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.portfolios[0].name, "Committed");
    assert_eq!(
        fixture.state.snapshot().await.unwrap().portfolios[0].name,
        "Committed"
    );
}

fn empty_batch() -> PriceBatch {
    PriceBatch {
        prices: Vec::new(),
        summary: SyncSummary {
            updated: 0,
            unsupported: 0,
        },
    }
}

#[tokio::test]
async fn manual_and_background_refreshes_share_a_single_slot_without_blocking_edits() {
    let fixture = Fixture::new();
    let (job, resume) = start_blocked_refresh(&fixture.state).await;
    assert!(fixture.state.is_refreshing());
    assert!(matches!(
        fixture.state.refresh_prices().await.unwrap(),
        PriceRefresh::AlreadyRunning
    ));
    fixture
        .state
        .edit(|store| {
            store.data.config.selected_base_currency = "USD".into();
            Ok(())
        })
        .await
        .unwrap();
    resume.send(()).unwrap();
    assert!(matches!(
        job.await.unwrap().unwrap(),
        PriceRefresh::Updated(_)
    ));
    assert!(!fixture.state.is_refreshing());
    assert_eq!(
        fixture
            .state
            .snapshot()
            .await
            .unwrap()
            .config
            .selected_base_currency,
        "USD"
    );
}

type RefreshJob = tokio::task::JoinHandle<Result<PriceRefresh, super::super::error::WebError>>;

async fn start_blocked_refresh(state: &AppState) -> (RefreshJob, mpsc::Sender<()>) {
    let (started, ready) = tokio::sync::oneshot::channel();
    let (resume, resumed) = mpsc::channel();
    let state = state.clone();
    let job = tokio::spawn(async move {
        state
            .refresh_with(move |_| {
                started.send(()).unwrap();
                resumed.recv_timeout(Duration::from_secs(5))?;
                Ok(empty_batch())
            })
            .await
    });
    ready.await.unwrap();
    (job, resume)
}

#[tokio::test]
async fn provider_failure_retains_data_and_releases_the_refresh_slot() {
    let fixture = Fixture::new();
    seed_prunable_history(&fixture.state).await;
    let before = serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap();
    let error = fixture
        .state
        .refresh_with(|_| anyhow::bail!("provider unavailable"))
        .await
        .err()
        .unwrap();
    assert_eq!(error.status, StatusCode::BAD_GATEWAY);
    assert!(!fixture.state.is_refreshing());
    assert_eq!(
        serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
        before
    );
    assert!(matches!(
        fixture
            .state
            .refresh_with(|_| Ok(empty_batch()))
            .await
            .unwrap(),
        PriceRefresh::Updated(_)
    ));
}

#[tokio::test]
async fn save_failure_rolls_back_and_releases_the_refresh_slot() {
    let fixture = Fixture::new();
    seed_prunable_history(&fixture.state).await;
    let before = serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap();
    std::fs::rename(
        fixture.directory.path().join("data"),
        fixture.directory.path().join("archive"),
    )
    .unwrap();
    std::fs::File::create(fixture.directory.path().join("data")).unwrap();
    let error = fixture
        .state
        .refresh_with(|_| Ok(empty_batch()))
        .await
        .err()
        .unwrap();
    assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!fixture.state.is_refreshing());
    assert_eq!(
        serde_json::to_value(fixture.state.snapshot().await.unwrap()).unwrap(),
        before
    );
}

async fn seed_prunable_history(state: &AppState) {
    state
        .edit(|store| {
            let id = crate::assets::save_asset(
                store,
                None,
                crate::assets::AssetInput {
                    symbol: "EUR".into(),
                    name: "Euro".into(),
                    kind: crate::model::AssetKind::Fiat,
                    yahoo_symbol: None,
                    tradingview_symbol: None,
                    valuation_currency: None,
                },
            )?;
            for hour in [1, 8, 9, 23] {
                store.data.prices.push(crate::model::Price {
                    asset_id: id,
                    timestamp: format!("2020-01-01T{hour:02}:00:00Z").parse().unwrap(),
                    price: rust_decimal::Decimal::ONE,
                    currency: "USD".into(),
                    source: "synthetic".into(),
                });
            }
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn successful_refresh_saves_fetched_prices_and_compaction_atomically() {
    let fixture = Fixture::new();
    seed_prunable_history(&fixture.state).await;
    let refresh = fixture
        .state
        .refresh_with(|data| {
            Ok(PriceBatch {
                prices: vec![crate::model::Price {
                    asset_id: data.assets[0].id,
                    timestamp: chrono::Utc::now(),
                    price: rust_decimal::Decimal::ONE,
                    currency: "USD".into(),
                    source: "yahoo".into(),
                }],
                summary: SyncSummary {
                    updated: 1,
                    unsupported: 0,
                },
            })
        })
        .await
        .unwrap();
    let PriceRefresh::Updated(summary) = refresh else {
        panic!("refresh did not run")
    };
    assert_eq!(summary.updated, 1);
    let data = fixture.state.snapshot().await.unwrap();
    assert_eq!(data.prices.len(), 2);
    assert_eq!(
        data.prices[0].timestamp,
        "2020-01-01T23:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
    );
    let saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(fixture.directory.path().join("data/store.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved, serde_json::to_value(data).unwrap());
    assert!(!fixture.state.is_refreshing());
}

#[tokio::test]
async fn refresh_compaction_preserves_transactions_added_while_fetch_was_blocked() {
    let fixture = Fixture::new();
    seed_prunable_history(&fixture.state).await;
    let (job, resume) = start_blocked_refresh(&fixture.state).await;
    fixture
        .state
        .edit(|store| {
            let portfolio = crate::portfolios::create_portfolio(
                store,
                crate::portfolios::PortfolioInput {
                    name: "Main".into(),
                },
            )?;
            crate::transactions::add_manual_transaction(
                store,
                crate::transactions::ManualTransactionInput {
                    portfolio_id: portfolio,
                    timestamp: "2020-01-01T10:00:00Z".parse().unwrap(),
                    kind: crate::model::TransactionKind::Deposit,
                    base_asset_id: store.data.assets[0].id,
                    base_amount: rust_decimal::Decimal::ONE,
                    quote_asset_id: None,
                    quote_amount: None,
                    quote_ledger_effect: crate::model::LedgerEffect::Ignore,
                    fee_asset_id: None,
                    fee_amount: None,
                    exchange: None,
                    broker: None,
                    notes: None,
                },
            )?;
            Ok(())
        })
        .await
        .unwrap();
    resume.send(()).unwrap();
    job.await.unwrap().unwrap();
    let data = fixture.state.snapshot().await.unwrap();
    assert_eq!(data.prices.len(), 2);
    assert_eq!(data.transactions.len(), 1);
    assert!(data.prices.iter().any(|p| {
        p.timestamp
            == "2020-01-01T09:00:00Z"
                .parse::<chrono::DateTime<chrono::Utc>>()
                .unwrap()
    }));
}
