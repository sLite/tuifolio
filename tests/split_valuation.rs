pub mod support;

use rust_decimal::Decimal;
use support::{create_asset, create_portfolio, temp_store};
use tuifolio::{accounting::build_report, ledger::rebuild_ledger, model::*, store::Store};

fn fixture() -> (Store, Id) {
    let mut store = temp_store();
    let portfolio = create_portfolio(&mut store, "Main");
    let asset = create_asset(&mut store, "ABC", "ABC", AssetKind::Stock);
    let id = store.data.allocate_id();
    store.data.transactions.push(Transaction {
        id,
        portfolio_id: portfolio,
        timestamp: "2020-01-01T00:00:00Z".parse().unwrap(),
        kind: TransactionKind::Gift,
        base_asset_id: asset,
        base_amount: 10.into(),
        quote_asset_id: None,
        quote_amount: None,
        quote_ledger_effect: LedgerEffect::Ignore,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
        source: "fixture".into(),
        source_row_hash: "fixture:gift".into(),
    });
    (store, asset)
}

#[test]
fn forward_reverse_and_compound_splits_preserve_value_from_pre_split_observations() {
    for splits in [vec![(2, 1)], vec![(1, 2)], vec![(2, 1), (1, 2)]] {
        let (mut store, asset) = fixture();
        for (index, (numerator, denominator)) in splits.into_iter().enumerate() {
            store.data.config.stock_splits.push(StockSplit {
                asset_id: asset,
                effective_date: format!("202{}-01-01", index + 1),
                numerator: numerator.into(),
                denominator: denominator.into(),
            });
        }
        store.data.prices.push(Price {
            asset_id: asset,
            timestamp: "2020-06-01T00:00:00Z".parse().unwrap(),
            price: 100.into(),
            currency: "EUR".into(),
            source: "fixture".into(),
        });
        rebuild_ledger(&mut store.data).unwrap();
        let before = serde_json::to_value(&store.data).unwrap();
        let report = build_report(&store.data);
        assert_eq!(report.net_value, Decimal::from(1000));
        assert_eq!(serde_json::to_value(&store.data).unwrap(), before);
    }
}

#[test]
fn a_post_split_observation_is_not_adjusted_again() {
    let (mut store, asset) = fixture();
    store.data.config.stock_splits.push(StockSplit {
        asset_id: asset,
        effective_date: "2021-01-01".into(),
        numerator: 2.into(),
        denominator: Decimal::ONE,
    });
    store.data.prices.push(Price {
        asset_id: asset,
        timestamp: "2021-01-01T00:00:00Z".parse().unwrap(),
        price: 50.into(),
        currency: "EUR".into(),
        source: "fixture".into(),
    });
    rebuild_ledger(&mut store.data).unwrap();
    assert_eq!(build_report(&store.data).net_value, Decimal::from(1000));
}

#[test]
fn split_adjusted_market_price_still_converts_to_selected_reporting_currency() {
    let (mut store, asset) = fixture();
    let usd = create_asset(&mut store, "USD", "Dollar", AssetKind::Fiat);
    store.data.config.stock_splits.push(StockSplit {
        asset_id: asset,
        effective_date: "2021-01-01".into(),
        numerator: 2.into(),
        denominator: Decimal::ONE,
    });
    for (asset_id, price, currency) in [
        (asset, Decimal::from(100), "USD"),
        (usd, Decimal::new(9, 1), "EUR"),
    ] {
        store.data.prices.push(Price {
            asset_id,
            timestamp: "2020-06-01T00:00:00Z".parse().unwrap(),
            price,
            currency: currency.into(),
            source: "fixture".into(),
        });
    }
    rebuild_ledger(&mut store.data).unwrap();
    assert_eq!(build_report(&store.data).net_value, Decimal::from(900));
}
