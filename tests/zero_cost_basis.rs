pub mod support;

use rust_decimal::Decimal;
use support::{create_asset, create_portfolio, temp_store};
use tuifolio::{
    accounting::build_report,
    assets::{AssetInput, save_asset},
    ledger::rebuild_ledger,
    model::{AssetKind, Id, LedgerEffect, LedgerRole, TransactionKind},
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction, update_transaction},
};

struct Fixture {
    store: Store,
    property: Id,
    quote: Id,
    portfolio: Id,
}

const NON_QUOTED_KINDS: [TransactionKind; 4] = [
    TransactionKind::Deposit,
    TransactionKind::Withdraw,
    TransactionKind::LiabilityIncrease,
    TransactionKind::LiabilityDecrease,
];

impl Fixture {
    fn new(currency: &str) -> Self {
        let mut store = temp_store();
        let property = save_asset(
            &mut store,
            None,
            AssetInput {
                symbol: "HOME".into(),
                name: "Gifted home".into(),
                kind: AssetKind::Property,
                yahoo_symbol: None,
                tradingview_symbol: None,
                valuation_currency: Some("EUR".into()),
            },
        )
        .unwrap();
        let quote = create_asset(&mut store, currency, currency, AssetKind::Fiat);
        let portfolio = create_portfolio(&mut store, "Main");
        Self {
            store,
            property,
            quote,
            portfolio,
        }
    }

    fn input(&self, kind: TransactionKind, amount: Option<Decimal>) -> ManualTransactionInput {
        ManualTransactionInput {
            portfolio_id: self.portfolio,
            timestamp: chrono::Utc::now(),
            kind,
            base_asset_id: self.property,
            base_amount: Decimal::from(350000),
            quote_asset_id: Some(self.quote),
            quote_amount: amount,
            quote_ledger_effect: LedgerEffect::Ignore,
            fee_asset_id: None,
            fee_amount: None,
            exchange: None,
            broker: None,
            notes: None,
        }
    }

    fn assert_zero_basis(&self) {
        let report = build_report(&self.store.data);
        assert_eq!(report.holdings.len(), 1);
        let holding = &report.holdings[0];
        assert_eq!(holding.asset_id, self.property);
        assert_eq!(holding.value, Some(Decimal::from(350000)));
        assert_eq!(holding.net_invested, Some(Decimal::ZERO));
        assert_eq!(holding.unrealized_pnl, holding.value);
        assert_eq!(report.total_unrealized_pnl, Some(Decimal::from(350000)));
    }
}

#[test]
fn gifted_property_has_a_known_zero_cost_basis_and_no_cash_movement() {
    let mut fixture = Fixture::new("EUR");
    let input = fixture.input(TransactionKind::AssetIncrease, Some(Decimal::ZERO));
    add_manual_transaction(&mut fixture.store, input).unwrap();
    assert_eq!(
        fixture.store.data.transactions[0].quote_amount,
        Some(Decimal::ZERO)
    );
    assert_eq!(fixture.store.data.ledger_entries.len(), 1);
    assert_eq!(
        fixture.store.data.ledger_entries[0].quantity_delta,
        Decimal::from(350000)
    );
    fixture.assert_zero_basis();
}

#[test]
fn blank_cost_basis_remains_unknown() {
    let mut fixture = Fixture::new("EUR");
    let input = fixture.input(TransactionKind::AssetIncrease, None);
    add_manual_transaction(&mut fixture.store, input).unwrap();
    let report = build_report(&fixture.store.data);
    assert_eq!(report.holdings[0].value, Some(Decimal::from(350000)));
    assert_eq!(report.holdings[0].net_invested, None);
    assert_eq!(report.holdings[0].unrealized_pnl, None);
}

#[test]
fn zero_cost_basis_does_not_require_a_foreign_exchange_rate() {
    let mut fixture = Fixture::new("USD");
    let input = fixture.input(TransactionKind::Buy, Some(Decimal::from(100)));
    let result = add_manual_transaction(&mut fixture.store, input).unwrap();
    assert_eq!(
        build_report(&fixture.store.data).holdings[0].net_invested,
        None
    );
    let input = fixture.input(TransactionKind::Buy, Some(Decimal::ZERO));
    update_transaction(&mut fixture.store, result.transaction_id, input).unwrap();
    assert!(fixture.store.data.prices.is_empty());
    fixture.assert_zero_basis();
}

#[test]
fn editing_a_paid_acquisition_to_zero_removes_its_cash_movement() {
    let mut fixture = Fixture::new("EUR");
    let mut input = fixture.input(TransactionKind::Buy, Some(Decimal::from(100)));
    input.quote_ledger_effect = LedgerEffect::Post;
    let result = add_manual_transaction(&mut fixture.store, input).unwrap();
    assert_eq!(fixture.store.data.ledger_entries.len(), 2);
    let previous = fixture.store.data.transactions[0].clone();
    let mut input = ManualTransactionInput::from(&previous);
    input.quote_amount = Some(Decimal::ZERO);
    update_transaction(&mut fixture.store, previous.id, input).unwrap();
    assert_eq!(fixture.store.data.transactions[0].id, result.transaction_id);
    assert_eq!(
        fixture.store.data.transactions[0].source_row_hash,
        previous.source_row_hash
    );
    assert_eq!(fixture.store.data.ledger_entries.len(), 1);
    fixture.assert_zero_basis();
}

#[test]
fn supported_transaction_kinds_accept_zero_quotes_without_posting_cash() {
    let kinds = [
        TransactionKind::Buy,
        TransactionKind::Sell,
        TransactionKind::AssetIncrease,
        TransactionKind::AssetDecrease,
    ];
    for kind in kinds {
        for effect in [LedgerEffect::Ignore, LedgerEffect::Post] {
            let mut fixture = Fixture::new("EUR");
            let mut input = fixture.input(kind, Some(Decimal::ZERO));
            input.quote_ledger_effect = effect;
            add_manual_transaction(&mut fixture.store, input).unwrap();
            assert_eq!(fixture.store.data.ledger_entries.len(), 1);
            assert!(matches!(
                fixture.store.data.ledger_entries[0].role,
                LedgerRole::Base
            ));
        }
    }
}

#[test]
fn unsupported_transaction_kinds_reject_quote_fields_without_mutating_data() {
    for kind in NON_QUOTED_KINDS {
        let mut fixture = Fixture::new("EUR");
        let before = serde_json::to_value(&fixture.store.data).unwrap();
        for amount in [None, Some(Decimal::ZERO), Some(Decimal::from(100))] {
            let input = fixture.input(kind, amount);
            let error = add_manual_transaction(&mut fixture.store, input).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("quote fields are only applicable")
            );
            assert_eq!(serde_json::to_value(&fixture.store.data).unwrap(), before);
        }
    }
}

#[test]
fn non_quoted_transactions_can_still_post_fees() {
    for kind in NON_QUOTED_KINDS {
        let mut fixture = Fixture::new("EUR");
        let mut input = fixture.input(kind, None);
        input.quote_asset_id = None;
        input.fee_asset_id = Some(fixture.quote);
        input.fee_amount = Some(Decimal::from(2));
        add_manual_transaction(&mut fixture.store, input).unwrap();
        assert_eq!(fixture.store.data.ledger_entries.len(), 2);
        let fee = &fixture.store.data.ledger_entries[1];
        assert!(matches!(fee.role, LedgerRole::Fee));
        assert_eq!(fee.asset_id, fixture.quote);
        assert_eq!(fee.quantity_delta, Decimal::from(-2));
    }
}

#[test]
fn ignored_historical_quotes_do_not_establish_a_zero_cost_basis() {
    for kind in NON_QUOTED_KINDS {
        let mut fixture = Fixture::new("EUR");
        let input = fixture.input(TransactionKind::AssetIncrease, Some(Decimal::from(100)));
        add_manual_transaction(&mut fixture.store, input).unwrap();
        fixture.store.data.transactions[0].kind = kind;
        rebuild_ledger(&mut fixture.store.data).unwrap();
        let report = build_report(&fixture.store.data);
        assert_eq!(report.holdings[0].net_invested, None);
        assert_eq!(report.holdings[0].unrealized_pnl, None);
        assert_eq!(
            fixture.store.data.transactions[0].quote_amount,
            Some(Decimal::from(100))
        );
    }
}

#[test]
fn unsupported_historical_quotes_do_not_require_exchange_rates_or_mask_known_costs() {
    let mut fixture = Fixture::new("EUR");
    let usd = create_asset(&mut fixture.store, "USD", "US dollar", AssetKind::Fiat);
    let acquisition = fixture.input(TransactionKind::Buy, Some(Decimal::from(100)));
    add_manual_transaction(&mut fixture.store, acquisition).unwrap();
    let movement = fixture.input(TransactionKind::AssetIncrease, Some(Decimal::ZERO));
    add_manual_transaction(&mut fixture.store, movement).unwrap();
    let previous = &mut fixture.store.data.transactions[1];
    previous.kind = TransactionKind::Deposit;
    previous.quote_asset_id = Some(usd);
    previous.quote_amount = Some(Decimal::from(777));
    rebuild_ledger(&mut fixture.store.data).unwrap();
    let report = build_report(&fixture.store.data);
    assert_eq!(report.holdings[0].net_invested, Some(Decimal::from(100)));
    assert_eq!(
        report.holdings[0].unrealized_pnl,
        Some(Decimal::from(699900))
    );
    assert!(fixture.store.data.prices.is_empty());
}

#[test]
fn negative_quotes_and_zero_quotes_without_a_currency_are_rejected() {
    let mut fixture = Fixture::new("EUR");
    let negative = fixture.input(TransactionKind::AssetIncrease, Some(Decimal::from(-1)));
    let mut missing_currency = fixture.input(TransactionKind::AssetIncrease, Some(Decimal::ZERO));
    missing_currency.quote_asset_id = None;
    for input in [negative, missing_currency] {
        assert!(add_manual_transaction(&mut fixture.store, input).is_err());
        assert!(fixture.store.data.transactions.is_empty());
        assert!(fixture.store.data.ledger_entries.is_empty());
    }
}
