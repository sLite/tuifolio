pub mod support;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use support::{create_asset, create_portfolio, temp_store};
use tuifolio::{
    accounting::build_report,
    model::{AssetKind, Id, LedgerEffect, TransactionKind},
    price_sync::add_manual_price_for_asset_at,
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction, update_transaction},
};

struct Fixture {
    store: Store,
    p: Id,
    sol: Id,
    eur: Id,
    eth: Id,
}
impl Fixture {
    fn new() -> Self {
        let mut store = temp_store();
        let p = create_portfolio(&mut store, "Main");
        let eur = create_asset(&mut store, "EUR", "Euro", AssetKind::Fiat);
        let usd = create_asset(&mut store, "USD", "Dollar", AssetKind::Fiat);
        let btc = create_asset(&mut store, "BTC", "Bitcoin", AssetKind::Crypto);
        let eth = create_asset(&mut store, "ETH", "Ether", AssetKind::Crypto);
        let sol = create_asset(&mut store, "SOL", "Solana", AssetKind::Crypto);
        for (id, price, currency) in [
            (eur, dec!(2), "USD"),
            (usd, dec!(0.5), "EUR"),
            (btc, dec!(100), "USD"),
            (eth, dec!(10), "USD"),
            (usd, dec!(0.01), "BTC"),
            (usd, dec!(0.1), "ETH"),
            (sol, dec!(20), "USD"),
        ] {
            add_manual_price_for_asset_at(
                &mut store,
                id,
                price,
                currency,
                "2020-06-01T00:00:00Z".parse().unwrap(),
            )
            .unwrap();
        }
        Self {
            store,
            p,
            sol,
            eur,
            eth,
        }
    }
    fn input(&self, kind: TransactionKind, qty: Decimal) -> ManualTransactionInput {
        ManualTransactionInput {
            portfolio_id: self.p,
            timestamp: "2020-06-02T00:00:00Z".parse().unwrap(),
            kind,
            base_asset_id: self.sol,
            base_amount: qty,
            quote_asset_id: None,
            quote_amount: None,
            quote_ledger_effect: LedgerEffect::Ignore,
            fee_asset_id: None,
            fee_amount: None,
            exchange: None,
            broker: None,
            notes: Some("staking reward".into()),
        }
    }
    fn reward(&mut self) -> Id {
        let input = self.input(TransactionKind::StakingReward, dec!(2));
        add_manual_transaction(&mut self.store, input)
            .unwrap()
            .transaction_id
    }
    fn mark(&mut self) {
        add_manual_price_for_asset_at(
            &mut self.store,
            self.sol,
            dec!(30),
            "USD",
            "2020-06-03T00:00:00Z".parse().unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn reward_zero_basis_and_full_value_pnl_work_in_every_reporting_currency() {
    let mut f = Fixture::new();
    f.reward();
    f.mark();
    for (base, value) in [
        ("EUR", dec!(30)),
        ("USD", dec!(60)),
        ("BTC", dec!(0.6)),
        ("ETH", dec!(6)),
    ] {
        f.store.data.config.selected_base_currency = base.into();
        let r = build_report(&f.store.data);
        let h = r.holdings.iter().find(|h| h.asset_id == f.sol).unwrap();
        assert_eq!(h.remaining_cost_basis, Some(dec!(0)));
        assert_eq!(h.unrealized_pnl, Some(value));
        assert_eq!(h.net_invested, Some(dec!(0)));
        assert_eq!(r.unresolved_pnl, 0);
    }
}

#[test]
fn rewards_add_units_without_increasing_paid_weighted_average_basis() {
    let mut f = Fixture::new();
    let mut buy = f.input(TransactionKind::Buy, dec!(2));
    buy.timestamp = "2020-06-01T12:00:00Z".parse().unwrap();
    buy.quote_asset_id = Some(f.eur);
    buy.quote_amount = Some(dec!(10));
    add_manual_transaction(&mut f.store, buy).unwrap();
    f.reward();
    f.mark();
    let r = build_report(&f.store.data);
    let h = r.holdings.iter().find(|h| h.asset_id == f.sol).unwrap();
    assert_eq!(h.quantity, dec!(4));
    assert_eq!(h.remaining_cost_basis, Some(dec!(10)));
    assert_eq!(h.unrealized_pnl, Some(dec!(50)));
    assert_eq!(h.net_invested, Some(dec!(10)));
    let mut sell = f.input(TransactionKind::Sell, dec!(1));
    sell.timestamp = "2020-06-04T00:00:00Z".parse().unwrap();
    sell.quote_asset_id = Some(f.eur);
    sell.quote_amount = Some(dec!(10));
    add_manual_transaction(&mut f.store, sell).unwrap();
    let r = build_report(&f.store.data);
    let h = r.holdings.iter().find(|h| h.asset_id == f.sol).unwrap();
    assert_eq!(h.remaining_cost_basis, Some(dec!(7.5)));
    assert_eq!(r.total_realized_pnl, Some(dec!(7.5)));
    assert_eq!(h.unrealized_pnl, Some(dec!(37.5)));
}

#[test]
fn closed_reward_position_realizes_full_sale_proceeds_without_receipt_income() {
    let mut f = Fixture::new();
    f.reward();
    let mut sell = f.input(TransactionKind::Sell, dec!(2));
    sell.timestamp = "2020-06-03T00:00:00Z".parse().unwrap();
    sell.quote_asset_id = Some(f.eur);
    sell.quote_amount = Some(dec!(40));
    add_manual_transaction(&mut f.store, sell).unwrap();
    let r = build_report(&f.store.data);
    assert!(!r.holdings.iter().any(|h| h.asset_id == f.sol));
    assert_eq!(r.total_realized_pnl, Some(dec!(40)));
    assert_eq!(r.realized_returns.len(), 1);
}

#[test]
fn missing_receipt_quotes_do_not_make_zero_basis_unknown() {
    let mut f = Fixture::new();
    f.store.data.prices.retain(|p| p.asset_id != f.sol);
    f.reward();
    let r = build_report(&f.store.data);
    let h = r.holdings.iter().find(|h| h.asset_id == f.sol).unwrap();
    assert_eq!(h.remaining_cost_basis, Some(dec!(0)));
    assert_eq!(h.net_invested, Some(dec!(0)));
    assert_eq!(h.value, None);
    assert_eq!(h.unrealized_pnl, None);
    f.mark();
    let r = build_report(&f.store.data);
    let h = r.holdings.iter().find(|h| h.asset_id == f.sol).unwrap();
    assert_eq!(h.remaining_cost_basis, Some(dec!(0)));
    assert_eq!(h.unrealized_pnl, Some(dec!(30)));
}

#[test]
fn receipt_needs_no_fx_but_current_market_value_still_does() {
    let mut f = Fixture::new();
    f.store.data.prices.clear();
    f.reward();
    let usd = f.store.asset_by_symbol("USD").unwrap().id;
    f.mark();
    let r = build_report(&f.store.data);
    assert_eq!(r.holdings[0].remaining_cost_basis, Some(dec!(0)));
    assert_eq!(r.holdings[0].unrealized_pnl, None);
    add_manual_price_for_asset_at(
        &mut f.store,
        usd,
        dec!(0.5),
        "EUR",
        "2020-06-03T00:00:00Z".parse().unwrap(),
    )
    .unwrap();
    let r = build_report(&f.store.data);
    assert_eq!(r.holdings[0].remaining_cost_basis, Some(dec!(0)));
    assert_eq!(r.holdings[0].unrealized_pnl, Some(dec!(30)));
}

#[test]
fn same_asset_fees_reduce_zero_cost_units_and_separate_fees_keep_existing_cost_policy() {
    for same_asset in [false, true] {
        let mut f = Fixture::new();
        let mut input = f.input(TransactionKind::StakingReward, dec!(2));
        input.fee_asset_id = Some(if same_asset { f.sol } else { f.eur });
        input.fee_amount = Some(if same_asset { dec!(0.2) } else { dec!(2) });
        add_manual_transaction(&mut f.store, input).unwrap();
        let r = build_report(&f.store.data);
        let h = r.holdings.iter().find(|h| h.asset_id == f.sol).unwrap();
        assert_eq!(h.quantity, if same_asset { dec!(1.8) } else { dec!(2) });
        assert_eq!(
            h.remaining_cost_basis,
            Some(if same_asset { dec!(0) } else { dec!(2) })
        );
        assert_eq!(h.unrealized_pnl, Some(dec!(18)));
        assert_eq!(r.total_realized_pnl, Some(dec!(0)));
    }
}

#[test]
fn third_investment_asset_fee_retains_its_own_basis_disposal() {
    let mut f = Fixture::new();
    let mut buy = f.input(TransactionKind::Buy, dec!(1));
    buy.base_asset_id = f.eth;
    buy.timestamp = "2020-06-01T12:00:00Z".parse().unwrap();
    buy.quote_asset_id = Some(f.eur);
    buy.quote_amount = Some(dec!(2));
    add_manual_transaction(&mut f.store, buy).unwrap();
    let mut input = f.input(TransactionKind::StakingReward, dec!(2));
    input.fee_asset_id = Some(f.eth);
    input.fee_amount = Some(dec!(0.1));
    add_manual_transaction(&mut f.store, input).unwrap();
    let r = build_report(&f.store.data);
    assert_eq!(
        r.holdings
            .iter()
            .find(|h| h.asset_id == f.sol)
            .unwrap()
            .remaining_cost_basis,
        Some(dec!(0.5))
    );
    assert_eq!(r.total_realized_pnl, Some(dec!(0.3)));
}

#[test]
fn noncrypto_rewards_payments_and_fully_consuming_fees_are_rejected_atomically() {
    let mut f = Fixture::new();
    let original = serde_json::to_value(&f.store.data).unwrap();
    for variant in 0..3 {
        let mut i = f.input(TransactionKind::StakingReward, dec!(2));
        match variant {
            0 => i.base_asset_id = f.eur,
            1 => {
                i.quote_asset_id = Some(f.eur);
                i.quote_amount = Some(dec!(2));
            }
            _ => {
                i.fee_asset_id = Some(f.sol);
                i.fee_amount = Some(dec!(2));
            }
        }
        assert!(add_manual_transaction(&mut f.store, i).is_err());
        assert_eq!(serde_json::to_value(&f.store.data).unwrap(), original);
    }
}

#[test]
fn changing_only_kind_preserves_quantity_postings_and_ignored_quote_metadata() {
    let mut f = Fixture::new();
    let mut input = f.input(TransactionKind::Deposit, dec!(2));
    let id = add_manual_transaction(&mut f.store, input.clone())
        .unwrap()
        .transaction_id;
    let ledger = serde_json::to_value(&f.store.data.ledger_entries).unwrap();
    input.kind = TransactionKind::StakingReward;
    input.quote_asset_id = Some(f.eur);
    update_transaction(&mut f.store, id, input).unwrap();
    assert_eq!(
        serde_json::to_value(&f.store.data.ledger_entries).unwrap(),
        ledger
    );
    assert_eq!(f.store.data.transactions[0].quote_asset_id, Some(f.eur));
    assert_eq!(
        build_report(&f.store.data).holdings[0].remaining_cost_basis,
        Some(dec!(0))
    );
}

#[test]
fn saved_reward_reopens_without_rewriting_metadata_or_zero_basis() {
    let mut f = Fixture::new();
    let id = f.reward();
    f.store
        .data
        .transactions
        .iter_mut()
        .find(|t| t.id == id)
        .unwrap()
        .quote_asset_id = Some(f.eur);
    f.store.save().unwrap();
    let path = f.store.path().clone();
    let expected = serde_json::to_value(&f.store.data).unwrap();
    drop(f.store);
    let reopened = Store::open(Some(path)).unwrap();
    assert_eq!(serde_json::to_value(&reopened.data).unwrap(), expected);
    assert_eq!(
        build_report(&reopened.data).holdings[0].remaining_cost_basis,
        Some(dec!(0))
    );
}

#[test]
fn successive_rewards_ignore_changes_in_receipt_market_value() {
    let mut f = Fixture::new();
    f.reward();
    f.mark();
    let mut next = f.input(TransactionKind::StakingReward, dec!(2));
    next.timestamp = "2020-06-04T00:00:00Z".parse().unwrap();
    add_manual_transaction(&mut f.store, next).unwrap();
    let r = build_report(&f.store.data);
    assert_eq!(r.holdings[0].remaining_cost_basis, Some(dec!(0)));
    assert_eq!(r.holdings[0].unrealized_pnl, Some(dec!(60)));
}

#[test]
fn zero_cost_rewards_do_not_resolve_an_uncosted_external_deposit() {
    let mut f = Fixture::new();
    let input = f.input(TransactionKind::Deposit, dec!(1));
    add_manual_transaction(&mut f.store, input).unwrap();
    f.reward();
    let r = build_report(&f.store.data);
    assert_eq!(r.holdings[0].remaining_cost_basis, None);
    assert_eq!(r.unresolved_pnl, 1);
}

#[test]
fn explicitly_worthless_rewards_have_known_zero_basis_and_pnl() {
    let mut f = Fixture::new();
    add_manual_price_for_asset_at(
        &mut f.store,
        f.sol,
        dec!(0),
        "USD",
        "2020-06-01T12:00:00Z".parse().unwrap(),
    )
    .unwrap();
    f.reward();
    let r = build_report(&f.store.data);
    assert_eq!(r.holdings[0].remaining_cost_basis, Some(dec!(0)));
    assert_eq!(r.holdings[0].unrealized_pnl, Some(dec!(0)));
    assert_eq!(r.unresolved_pnl, 0);
}
