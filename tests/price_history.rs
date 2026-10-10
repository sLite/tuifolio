pub mod support;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::BTreeMap;
use tuifolio::{
    accounting::build_report,
    model::{AssetKind, Id, LedgerEffect, Price, StoreData, Transaction, TransactionKind},
    price_history::compact_price_history,
    price_sync::{PriceBatch, SyncSummary, merge_price_batch},
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}
fn price(id: Id, time: &str, currency: &str, value: Decimal) -> Price {
    Price {
        asset_id: id,
        timestamp: at(time),
        currency: currency.into(),
        price: value,
        source: "yahoo".into(),
    }
}
fn transaction(id: Id, time: &str) -> Transaction {
    Transaction {
        id,
        portfolio_id: 1,
        timestamp: at(time),
        kind: TransactionKind::Deposit,
        base_asset_id: 1,
        base_amount: dec!(1),
        quote_asset_id: None,
        quote_amount: None,
        quote_ledger_effect: LedgerEffect::Ignore,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: None,
        source: "synthetic".into(),
        source_row_hash: format!("synthetic:{id}"),
    }
}
fn snapshot(
    data: &StoreData,
    cutoff: Option<DateTime<Utc>>,
) -> BTreeMap<(Id, String), serde_json::Value> {
    let mut result: BTreeMap<(Id, String), (DateTime<Utc>, serde_json::Value)> = BTreeMap::new();
    for p in &data.prices {
        if cutoff.is_some_and(|t| p.timestamp > t) {
            continue;
        }
        let code = if p.currency == "GBp" || p.currency.trim().eq_ignore_ascii_case("GBX") {
            "GBP".into()
        } else {
            p.currency.trim().to_ascii_uppercase()
        };
        let k = (p.asset_id, code);
        if result.get(&k).is_none_or(|(t, _)| *t < p.timestamp) {
            result.insert(k, (p.timestamp, serde_json::to_value(p).unwrap()));
        }
    }
    result.into_iter().map(|(k, (_, p))| (k, p)).collect()
}

#[test]
fn completed_days_keep_daily_last_per_asset_id_and_quote_currency() {
    let mut d = StoreData::default();
    for id in [1, 2] {
        for c in ["EUR", "USD"] {
            for t in [
                "2020-01-01T01:00:00Z",
                "2020-01-01T23:00:00Z",
                "2020-01-02T09:00:00Z",
            ] {
                d.prices.push(price(id, t, c, dec!(1)));
            }
        }
    }
    let latest = snapshot(&d, None);
    assert_eq!(compact_price_history(&mut d, at("2020-01-03T00:00:00Z")), 4);
    assert_eq!(d.prices.len(), 8);
    assert_eq!(snapshot(&d, None), latest);
    assert!(
        d.prices
            .iter()
            .all(|p| p.timestamp != at("2020-01-01T01:00:00Z"))
    );
}

#[test]
fn intraday_cutoffs_keep_predecessors_including_exact_time_and_previous_day() {
    let mut d = StoreData::default();
    for t in [
        "2020-01-01T23:00:00Z",
        "2020-01-02T08:00:00Z",
        "2020-01-02T09:00:00Z",
        "2020-01-02T12:00:00Z",
        "2020-01-02T23:00:00Z",
    ] {
        d.prices.push(price(1, t, "USD", dec!(2)));
    }
    for (i, t) in [
        "2019-12-31T00:00:00Z",
        "2020-01-02T01:00:00Z",
        "2020-01-02T10:00:00Z",
        "2020-01-02T12:00:00Z",
    ]
    .into_iter()
    .enumerate()
    {
        d.transactions.push(transaction(i as Id + 1, t));
    }
    let expected: Vec<_> = d
        .transactions
        .iter()
        .map(|t| snapshot(&d, Some(t.timestamp)))
        .collect();
    assert_eq!(compact_price_history(&mut d, at("2020-01-03T00:00:00Z")), 1);
    for (t, x) in d.transactions.iter().zip(expected) {
        assert_eq!(snapshot(&d, Some(t.timestamp)), x);
    }
    assert!(
        !d.prices
            .iter()
            .any(|p| p.timestamp == at("2020-01-02T08:00:00Z"))
    );
}

#[test]
fn current_utc_day_and_future_records_remain_intraday_until_rollover() {
    let mut d = StoreData::default();
    for t in [
        "2020-01-01T23:59:00Z",
        "2020-01-02T00:00:00Z",
        "2020-01-02T00:05:00Z",
        "2020-01-03T00:00:00Z",
    ] {
        d.prices.push(price(1, t, "EUR", dec!(0)));
    }
    assert_eq!(compact_price_history(&mut d, at("2020-01-02T20:00:00Z")), 0);
    assert_eq!(compact_price_history(&mut d, at("2020-01-03T00:01:00Z")), 1);
    assert_eq!(d.prices.len(), 3);
    assert!(
        d.prices
            .iter()
            .any(|p| p.timestamp == at("2020-01-02T00:05:00Z"))
    );
}

#[test]
fn equal_timestamp_winner_and_minor_unit_records_match_accounting_without_repricing() {
    let mut d = StoreData::default();
    let first = price(1, "2020-01-01T01:00:00Z", "GBp", dec!(123));
    let second = price(1, "2020-01-01T01:00:00Z", "GBP", dec!(100));
    d.prices = vec![
        first.clone(),
        second,
        price(1, "2020-01-01T23:00:00Z", "GBX", dec!(456)),
    ];
    d.transactions.push(transaction(1, "2020-01-01T02:00:00Z"));
    let expected = snapshot(&d, Some(d.transactions[0].timestamp));
    assert_eq!(compact_price_history(&mut d, at("2020-01-02T00:00:00Z")), 1);
    assert_eq!(
        serde_json::to_value(&d.prices[0]).unwrap(),
        serde_json::to_value(first).unwrap()
    );
    assert_eq!(snapshot(&d, Some(d.transactions[0].timestamp)), expected);
}

#[test]
fn source_labels_do_not_protect_unused_backfills_proxies_or_manual_records() {
    let mut d = StoreData::default();
    for (i, source) in [
        "manual",
        "backfill:research",
        "user-proxy:prelaunch",
        "yahoo",
    ]
    .into_iter()
    .enumerate()
    {
        let mut p = price(1, &format!("2020-01-01T0{}:00:00Z", i + 1), "USD", dec!(1));
        p.source = source.into();
        d.prices.push(p);
    }
    assert_eq!(compact_price_history(&mut d, at("2020-01-02T00:00:00Z")), 3);
    assert_eq!(d.prices[0].source, "yahoo");
}

#[test]
fn required_proxy_and_reciprocal_survive_by_cutoff_not_by_source() {
    let mut d = StoreData::default();
    for (id, c) in [(1, "USD"), (2, "ETH")] {
        for t in [
            "2013-01-01T00:00:00Z",
            "2013-01-01T01:00:00Z",
            "2013-01-01T23:00:00Z",
        ] {
            let mut p = price(id, t, c, dec!(2));
            p.source = "user-proxy:prelaunch".into();
            d.prices.push(p);
        }
    }
    d.transactions.push(transaction(1, "2013-01-01T00:00:00Z"));
    let expected = snapshot(&d, Some(d.transactions[0].timestamp));
    assert_eq!(compact_price_history(&mut d, at("2020-01-01T00:00:00Z")), 2);
    assert_eq!(snapshot(&d, Some(d.transactions[0].timestamp)), expected);
}

#[test]
fn unsorted_history_preserves_all_snapshots_global_latest_order_and_idempotence() {
    let mut d = StoreData::default();
    for n in (0..600).rev() {
        let mut p = price(
            n % 3 + 1,
            &format!("2020-01-{:02}T{:02}:{:02}:00Z", n / 60 + 1, n % 24, n % 60),
            if n % 2 == 0 { "usd" } else { "EUR" },
            Decimal::from(n),
        );
        p.source = format!("source:{n}");
        d.prices.push(p);
    }
    for n in 0..20 {
        d.transactions.push(transaction(
            n + 1,
            &format!("2020-01-{:02}T12:30:00Z", n % 10 + 1),
        ));
    }
    let original = d.clone();
    let latest = snapshot(&d, None);
    let expected: Vec<_> = d
        .transactions
        .iter()
        .map(|t| snapshot(&d, Some(t.timestamp)))
        .collect();
    let removed = compact_price_history(&mut d, at("2020-01-11T00:00:00Z"));
    assert!(removed > 400);
    assert_eq!(snapshot(&d, None), latest);
    for (t, x) in d.transactions.iter().zip(expected) {
        assert_eq!(snapshot(&d, Some(t.timestamp)), x);
    }
    let mut pos = 0;
    for p in &d.prices {
        while serde_json::to_value(&original.prices[pos]).unwrap()
            != serde_json::to_value(p).unwrap()
        {
            pos += 1;
        }
        pos += 1;
    }
    let once = serde_json::to_value(&d).unwrap();
    assert_eq!(compact_price_history(&mut d, at("2020-01-11T00:00:00Z")), 0);
    assert_eq!(serde_json::to_value(&d).unwrap(), once);
}

#[test]
fn refresh_merge_compacts_against_current_transactions_not_the_fetch_snapshot() {
    let mut d = StoreData::default();
    for t in [
        "2020-01-01T01:00:00Z",
        "2020-01-01T08:00:00Z",
        "2020-01-01T09:00:00Z",
        "2020-01-01T23:00:00Z",
    ] {
        d.prices.push(price(1, t, "EUR", dec!(1)));
    }
    let fetch_snapshot = d.clone();
    d.transactions.push(transaction(1, "2020-01-01T10:00:00Z"));
    let expected = snapshot(&d, Some(d.transactions[0].timestamp));
    let summary = merge_price_batch(
        &mut d,
        &fetch_snapshot,
        PriceBatch {
            prices: vec![],
            summary: SyncSummary {
                updated: 0,
                unsupported: 0,
            },
        },
    );
    assert_eq!(summary.updated, 0);
    assert_eq!(d.prices.len(), 2);
    assert_eq!(snapshot(&d, Some(d.transactions[0].timestamp)), expected);
}

#[test]
fn compaction_preserves_conversion_basis_realized_and_unrealized_in_four_bases_after_reopen() {
    use support::{create_asset, create_portfolio, temp_store};
    let mut s = temp_store();
    let portfolio = create_portfolio(&mut s, "Main");
    let stock = create_asset(&mut s, "ABC", "Stock", AssetKind::Stock);
    let eur = create_asset(&mut s, "EUR", "Euro", AssetKind::Fiat);
    let usd = create_asset(&mut s, "USD", "Dollar", AssetKind::Fiat);
    let btc = create_asset(&mut s, "BTC", "Bitcoin", AssetKind::Crypto);
    let eth = create_asset(&mut s, "ETH", "Ether", AssetKind::Crypto);
    for time in [
        "2020-01-01T00:00:00Z",
        "2020-01-01T08:00:00Z",
        "2020-01-01T09:00:00Z",
        "2020-01-01T12:00:00Z",
        "2020-01-01T23:00:00Z",
    ] {
        let changed = if time == "2020-01-01T23:00:00Z" {
            dec!(2)
        } else {
            dec!(1)
        };
        for (id, c, value) in [
            (stock, "USD", dec!(20)),
            (usd, "EUR", dec!(0.5)),
            (eur, "USD", dec!(2)),
            (btc, "USD", dec!(100)),
            (eth, "USD", dec!(10)),
            (usd, "BTC", dec!(0.01)),
            (usd, "ETH", dec!(0.1)),
        ] {
            s.data.prices.push(price(id, time, c, value * changed));
        }
    }
    for (time, kind, qty, value) in [
        (
            "2020-01-01T10:00:00Z",
            TransactionKind::Buy,
            dec!(4),
            dec!(40),
        ),
        (
            "2020-01-01T13:00:00Z",
            TransactionKind::Sell,
            dec!(1),
            dec!(20),
        ),
    ] {
        add_manual_transaction(
            &mut s,
            ManualTransactionInput {
                portfolio_id: portfolio,
                timestamp: at(time),
                kind,
                base_asset_id: stock,
                base_amount: qty,
                quote_asset_id: Some(usd),
                quote_amount: Some(value),
                quote_ledger_effect: LedgerEffect::Ignore,
                fee_asset_id: Some(usd),
                fee_amount: Some(dec!(1)),
                exchange: None,
                broker: None,
                notes: None,
            },
        )
        .unwrap();
    }
    let reports = |data: &StoreData| {
        ["EUR", "USD", "BTC", "ETH"].map(|b| {
            let mut x = data.clone();
            x.config.selected_base_currency = b.into();
            format!("{:?}", build_report(&x))
        })
    };
    let before = reports(&s.data);
    let mut other = serde_json::to_value(&s.data).unwrap();
    other.as_object_mut().unwrap().remove("prices");
    assert!(compact_price_history(&mut s.data, at("2020-01-02T00:00:00Z")) > 0);
    assert_eq!(reports(&s.data), before);
    let mut after = serde_json::to_value(&s.data).unwrap();
    after.as_object_mut().unwrap().remove("prices");
    assert_eq!(after, other);
    s.save().unwrap();
    let p = s.path().clone();
    let expected = serde_json::to_value(&s.data).unwrap();
    drop(s);
    let reopened = Store::open(Some(p)).unwrap();
    assert_eq!(serde_json::to_value(&reopened.data).unwrap(), expected);
    assert_eq!(reports(&reopened.data), before);
}

#[test]
fn cli_successful_refresh_compacts_and_saves_without_provider_requests() {
    use support::{create_asset, temp_store};
    let mut s = temp_store();
    let id = create_asset(&mut s, "HOUSE", "Property", AssetKind::Property);
    for time in [
        "2020-01-01T01:00:00Z",
        "2020-01-01T09:00:00Z",
        "2020-01-01T23:00:00Z",
    ] {
        s.data.prices.push(price(id, time, "EUR", dec!(10)));
    }
    s.save().unwrap();
    let mut expected = serde_json::to_value(&s.data).unwrap();
    expected["prices"] = serde_json::json!([s.data.prices.last().unwrap()]);
    let path = s.path().clone();
    drop(s);
    // A Property with no Yahoo mapping requests no quotes or FX.
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_tuifolio"))
        .args(["--store", path.to_str().unwrap(), "sync-prices"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result);
    let actual: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        serde_json::to_value(Store::open(Some(path)).unwrap().data).unwrap(),
        expected
    );
}

#[test]
fn empty_history_is_a_noop() {
    let mut d = StoreData::default();
    assert_eq!(compact_price_history(&mut d, Utc::now()), 0);
}
