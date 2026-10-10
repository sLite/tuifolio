pub mod support;

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use support::{create_asset, create_portfolio, temp_store};
use tuifolio::{
    accounting::{HoldingRow, build_report},
    model::{AssetKind, Id, LedgerEffect, TransactionKind},
    price_sync::add_manual_price_for_asset_at,
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

struct Fixture {
    store: Store,
    p: Id,
    eur: Id,
    usdc: Id,
    btc: Id,
}
impl Fixture {
    fn new() -> Self {
        let mut store = temp_store();
        let p = create_portfolio(&mut store, "Main");
        let eur = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
        let usdc = create_asset(&mut store, "USDC", "USDC", AssetKind::Crypto);
        let btc = create_asset(&mut store, "BTC", "Bitcoin", AssetKind::Crypto);
        for (asset, price) in [(usdc, dec!(1)), (btc, dec!(300))] {
            add_manual_price_for_asset_at(
                &mut store,
                asset,
                price,
                "EUR",
                "2020-01-01T00:00:00Z".parse().unwrap(),
            )
            .unwrap();
        }
        let mut f = Self {
            store,
            p,
            eur,
            usdc,
            btc,
        };
        f.post(
            TransactionKind::Buy,
            usdc,
            dec!(1000),
            Some((eur, dec!(800))),
            None,
            "2021-01-01T00:00:00Z",
        );
        f
    }
    fn post(
        &mut self,
        kind: TransactionKind,
        asset: Id,
        qty: Decimal,
        quote: Option<(Id, Decimal)>,
        fee: Option<(Id, Decimal)>,
        date: &str,
    ) {
        add_manual_transaction(
            &mut self.store,
            ManualTransactionInput {
                portfolio_id: self.p,
                timestamp: date.parse().unwrap(),
                kind,
                base_asset_id: asset,
                base_amount: qty,
                quote_asset_id: quote.map(|q| q.0),
                quote_amount: quote.map(|q| q.1),
                quote_ledger_effect: LedgerEffect::Post,
                fee_asset_id: fee.map(|q| q.0),
                fee_amount: fee.map(|q| q.1),
                exchange: None,
                broker: None,
                notes: None,
            },
        )
        .unwrap();
    }
    fn holding(&self, id: Id) -> HoldingRow {
        build_report(&self.store.data)
            .holdings
            .into_iter()
            .find(|h| h.asset_id == id)
            .unwrap()
    }
}

#[test]
fn bought_btc_consumes_usdc_basis_and_capitalizes_a_fee_paid_in_received_btc() {
    for fee in [dec!(0.2), dec!(0.05)] {
        let mut f = Fixture::new();
        f.post(
            TransactionKind::Buy,
            f.btc,
            dec!(2),
            Some((f.usdc, dec!(600))),
            Some((f.btc, fee)),
            "2022-01-01T00:00:00Z",
        );
        let btc = f.holding(f.btc);
        let usdc = f.holding(f.usdc);
        assert_eq!(btc.quantity, dec!(2) - fee);
        assert_eq!(btc.remaining_cost_basis, Some(dec!(600)));
        assert_eq!(btc.unrealized_pnl, Some(-fee * dec!(300)));
        assert_eq!(usdc.quantity, dec!(400));
        assert_eq!(usdc.remaining_cost_basis, Some(dec!(320)));
        assert_eq!(usdc.realized_pnl, Some(dec!(120)));
        assert_eq!(
            build_report(&f.store.data).total_realized_pnl,
            Some(dec!(120))
        );
    }
}

#[test]
fn equivalent_buy_and_sell_representations_produce_identical_basis_and_profit() {
    let mut buy = Fixture::new();
    let mut sell = Fixture::new();
    buy.post(
        TransactionKind::Buy,
        buy.btc,
        dec!(2),
        Some((buy.usdc, dec!(600))),
        Some((buy.btc, dec!(0.2))),
        "2022-01-01T00:00:00Z",
    );
    sell.post(
        TransactionKind::Sell,
        sell.usdc,
        dec!(600),
        Some((sell.btc, dec!(2))),
        Some((sell.btc, dec!(0.2))),
        "2022-01-01T00:00:00Z",
    );
    for id in [buy.btc, buy.usdc] {
        let a = buy.holding(id);
        let b = sell.holding(id);
        assert_eq!(
            (
                a.quantity,
                a.remaining_cost_basis,
                a.unrealized_pnl,
                a.realized_pnl
            ),
            (
                b.quantity,
                b.remaining_cost_basis,
                b.unrealized_pnl,
                b.realized_pnl
            )
        );
    }
}

#[test]
fn third_asset_fee_has_its_own_disposal_and_increases_acquisition_basis() {
    let mut f = Fixture::new();
    let eth = create_asset(&mut f.store, "ETH", "Ether", AssetKind::Crypto);
    add_manual_price_for_asset_at(
        &mut f.store,
        eth,
        dec!(50),
        "EUR",
        "2020-01-01T00:00:00Z".parse().unwrap(),
    )
    .unwrap();
    f.post(
        TransactionKind::Buy,
        eth,
        dec!(1),
        Some((f.eur, dec!(40))),
        None,
        "2021-01-02T00:00:00Z",
    );
    f.post(
        TransactionKind::Buy,
        f.btc,
        dec!(2),
        Some((f.usdc, dec!(600))),
        Some((eth, dec!(0.1))),
        "2022-01-01T00:00:00Z",
    );
    assert_eq!(f.holding(f.btc).remaining_cost_basis, Some(dec!(605)));
    let fee = f.holding(eth);
    assert_eq!(fee.quantity, dec!(0.9));
    assert_eq!(fee.remaining_cost_basis, Some(dec!(36)));
    assert_eq!(fee.realized_pnl, Some(dec!(1)));
}

#[test]
fn fiat_fees_raise_purchase_basis_and_reduce_sale_proceeds() {
    let mut f = Fixture::new();
    let stock = create_asset(&mut f.store, "ABC", "Stock", AssetKind::Stock);
    add_manual_price_for_asset_at(
        &mut f.store,
        stock,
        dec!(20),
        "EUR",
        "2020-01-01T00:00:00Z".parse().unwrap(),
    )
    .unwrap();
    f.post(
        TransactionKind::Buy,
        stock,
        dec!(10),
        Some((f.eur, dec!(100))),
        Some((f.eur, dec!(10))),
        "2021-01-02T00:00:00Z",
    );
    assert_eq!(f.holding(stock).remaining_cost_basis, Some(dec!(110)));
    f.post(
        TransactionKind::Sell,
        stock,
        dec!(5),
        Some((f.eur, dec!(100))),
        Some((f.eur, dec!(10))),
        "2022-01-01T00:00:00Z",
    );
    let h = f.holding(stock);
    assert_eq!(h.remaining_cost_basis, Some(dec!(55)));
    assert_eq!(h.realized_pnl, Some(dec!(35)));
    assert_eq!(h.unrealized_pnl, Some(dec!(45)));
}

#[test]
fn missing_disposal_fx_does_not_destroy_known_remaining_acquisition_basis() {
    let mut f = Fixture::new();
    let usd = create_asset(&mut f.store, "USD", "Dollar", AssetKind::Fiat);
    f.post(
        TransactionKind::Sell,
        f.usdc,
        dec!(600),
        Some((usd, dec!(600))),
        None,
        "2022-01-01T00:00:00Z",
    );
    let h = f.holding(f.usdc);
    assert_eq!(h.remaining_cost_basis, Some(dec!(320)));
    assert_eq!(h.unrealized_pnl, Some(dec!(80)));
    assert_eq!(h.realized_pnl, None);
    assert_eq!(build_report(&f.store.data).unresolved_realized_pnl, 1);
}

#[test]
fn unknown_receipts_remain_unknown_instead_of_borrowing_other_units_costs() {
    let mut f = Fixture::new();
    f.post(
        TransactionKind::Deposit,
        f.btc,
        dec!(2),
        None,
        None,
        "2021-01-02T00:00:00Z",
    );
    f.post(
        TransactionKind::Buy,
        f.btc,
        dec!(2),
        Some((f.usdc, dec!(600))),
        None,
        "2022-01-01T00:00:00Z",
    );
    assert_eq!(f.holding(f.btc).remaining_cost_basis, None);
    assert_eq!(f.holding(f.btc).unrealized_pnl, None);
}

#[test]
fn closed_disposal_survives_and_reopening_does_not_reuse_previous_profit() {
    let mut f = Fixture::new();
    let stock = create_asset(&mut f.store, "ABC", "Stock", AssetKind::Stock);
    add_manual_price_for_asset_at(
        &mut f.store,
        stock,
        dec!(20),
        "EUR",
        "2020-01-01T00:00:00Z".parse().unwrap(),
    )
    .unwrap();
    f.post(
        TransactionKind::Buy,
        stock,
        dec!(10),
        Some((f.eur, dec!(100))),
        None,
        "2021-01-02T00:00:00Z",
    );
    f.post(
        TransactionKind::Sell,
        stock,
        dec!(10),
        Some((f.eur, dec!(200))),
        None,
        "2022-01-01T00:00:00Z",
    );
    assert!(
        !build_report(&f.store.data)
            .holdings
            .iter()
            .any(|h| h.asset_id == stock)
    );
    assert_eq!(
        build_report(&f.store.data)
            .realized_returns
            .iter()
            .find(|r| r.asset_id == stock)
            .unwrap()
            .pnl,
        Some(dec!(100))
    );
    f.post(
        TransactionKind::Buy,
        stock,
        dec!(5),
        Some((f.eur, dec!(80))),
        None,
        "2023-01-01T00:00:00Z",
    );
    let h = f.holding(stock);
    assert_eq!(h.remaining_cost_basis, Some(dec!(80)));
    assert_eq!(h.unrealized_pnl, Some(dec!(20)));
    assert_eq!(h.realized_pnl, Some(dec!(100)));
}

#[test]
fn a_fee_in_the_funding_asset_consumes_additional_units_without_double_counting() {
    let mut f = Fixture::new();
    f.post(
        TransactionKind::Buy,
        f.btc,
        dec!(2),
        Some((f.usdc, dec!(600))),
        Some((f.usdc, dec!(5))),
        "2022-01-01T00:00:00Z",
    );
    assert_eq!(f.holding(f.btc).remaining_cost_basis, Some(dec!(605)));
    let usdc = f.holding(f.usdc);
    assert_eq!(usdc.quantity, dec!(395));
    assert_eq!(usdc.remaining_cost_basis, Some(dec!(316)));
    assert_eq!(usdc.realized_pnl, Some(dec!(121)));
}

#[test]
fn ignored_quote_information_does_not_dispose_owned_quote_units() {
    let mut f = Fixture::new();
    f.post(
        TransactionKind::Buy,
        f.btc,
        dec!(2),
        Some((f.usdc, dec!(600))),
        None,
        "2022-01-01T00:00:00Z",
    );
    f.store
        .data
        .transactions
        .last_mut()
        .unwrap()
        .quote_ledger_effect = LedgerEffect::Ignore;
    tuifolio::ledger::rebuild_ledger(&mut f.store.data).unwrap();
    let usdc = f.holding(f.usdc);
    assert_eq!(usdc.quantity, dec!(1000));
    assert_eq!(usdc.remaining_cost_basis, Some(dec!(800)));
    assert_eq!(usdc.realized_pnl, None);
    assert_eq!(f.holding(f.btc).remaining_cost_basis, Some(dec!(600)));
}

#[test]
fn a_zero_quote_is_zero_cost_even_when_the_received_asset_is_the_reporting_currency() {
    let mut f = Fixture::new();
    f.store.data.config.selected_base_currency = "BTC".into();
    f.post(
        TransactionKind::Buy,
        f.btc,
        dec!(1),
        Some((f.eur, dec!(0))),
        None,
        "2022-01-01T00:00:00Z",
    );
    assert_eq!(f.holding(f.btc).remaining_cost_basis, Some(dec!(0)));
    assert_eq!(f.holding(f.btc).unrealized_pnl, Some(dec!(1)));
}

#[test]
fn chronological_replay_is_independent_of_stored_vector_order_and_does_not_rewrite_records() {
    let mut f = Fixture::new();
    f.post(
        TransactionKind::Buy,
        f.btc,
        dec!(2),
        Some((f.usdc, dec!(600))),
        Some((f.btc, dec!(0.2))),
        "2022-01-01T00:00:00Z",
    );
    let expected = f.holding(f.usdc);
    f.store.data.transactions.reverse();
    let original = serde_json::to_value(&f.store.data).unwrap();
    assert_eq!(
        f.holding(f.usdc).remaining_cost_basis,
        expected.remaining_cost_basis
    );
    assert_eq!(serde_json::to_value(&f.store.data).unwrap(), original);
    f.store.save().unwrap();
    let path = f.store.path().clone();
    drop(f.store);
    let reopened = Store::open(Some(path)).unwrap();
    assert_eq!(
        build_report(&reopened.data)
            .holdings
            .iter()
            .find(|h| h.asset_id == f.usdc)
            .unwrap()
            .remaining_cost_basis,
        Some(dec!(320))
    );
}
