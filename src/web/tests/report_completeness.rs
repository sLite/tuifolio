use axum::http::StatusCode;
use rust_decimal::Decimal;

use super::fixture::{TestApp, configure_asset, html};
use crate::{
    assets::{AssetInput, save_asset},
    model::{AssetKind, Id, LedgerEffect, TransactionKind},
    portfolios::{PortfolioInput, create_portfolio},
    price_sync::add_manual_price,
    store::Store,
    transactions::{ManualTransactionInput, add_manual_transaction},
};

fn post(
    store: &mut Store,
    portfolio: Id,
    asset: Id,
    kind: TransactionKind,
    quantity: i64,
    cost: Option<i64>,
) {
    let euro = store.asset_by_symbol("EUR").unwrap().id;
    add_manual_transaction(
        store,
        ManualTransactionInput {
            portfolio_id: portfolio,
            timestamp: chrono::Utc::now(),
            kind,
            base_asset_id: asset,
            base_amount: Decimal::from(quantity),
            quote_asset_id: cost.map(|_| euro),
            quote_amount: cost.map(Decimal::from),
            quote_ledger_effect: LedgerEffect::Ignore,
            fee_asset_id: None,
            fee_amount: None,
            exchange: None,
            broker: None,
            notes: None,
        },
    )
    .unwrap();
}

fn investment(store: &mut Store, symbol: &str) -> Id {
    save_asset(
        store,
        None,
        AssetInput {
            symbol: symbol.into(),
            name: symbol.into(),
            kind: AssetKind::Custom,
            yahoo_symbol: None,
            tradingview_symbol: None,
            valuation_currency: Some("EUR".into()),
        },
    )
    .unwrap()
}

fn row(page: &str, portfolio: Id) -> &str {
    page.split("<tr>")
        .find(|row| row.contains(&format!("href=\"/portfolios/{portfolio}\"")))
        .unwrap()
}

#[tokio::test]
async fn unvalued_cash_and_debt_produce_partial_totals_and_no_false_complete_status() {
    let fixture = TestApp::with_assets();
    let (main, empty) = fixture
        .state
        .edit(|store| {
            let main = store.data.portfolios[0].id;
            let euro = store.asset_by_symbol("EUR").unwrap().id;
            let usd = store.asset_by_symbol("USD").unwrap().id;
            let debt = configure_asset(store, "DEBT", "Debt", AssetKind::Liability);
            post(store, main, euro, TransactionKind::Deposit, 1000, None);
            post(store, main, usd, TransactionKind::Deposit, 100, None);
            post(
                store,
                main,
                debt,
                TransactionKind::LiabilityIncrease,
                20,
                None,
            );
            let empty = create_portfolio(
                store,
                PortfolioInput {
                    name: "Empty".into(),
                },
            )?;
            Ok((main, empty))
        })
        .await
        .unwrap();
    let before = std::fs::read(&fixture.path).unwrap();
    for path in ["/".to_string(), format!("/portfolios/{main}")] {
        let page = html(fixture.get(&path).await, StatusCode::OK).await;
        for label in ["Net worth", "Assets", "Liabilities"] {
            assert!(page.contains(&format!("{label} · Partial</span>")));
        }
        assert!(page.contains("2 holdings have no valuation"));
        assert!(page.contains("including unvalued cash and liabilities"));
        for symbol in ["USD", "DEBT"] {
            let holding = page
                .split("<tr>")
                .find(|row| row.contains(&format!(">{symbol}</a>")))
                .unwrap();
            assert!(holding.contains("Unavailable"));
            assert!(holding.contains("Set up pricing"));
            assert!(holding.contains("warning-text"));
        }
    }
    let list = html(fixture.get("/portfolios").await, StatusCode::OK).await;
    let main_row = row(&list, main);
    assert!(main_row.contains("2 unvalued holdings"));
    assert!(!main_row.contains("Complete"));
    assert_eq!(main_row.matches(">Partial</span>").count(), 3);
    let empty_row = row(&list, empty);
    assert!(empty_row.contains("Complete"));
    assert!(!empty_row.contains("Partial"));
    let empty_page = html(
        fixture.get(&format!("/portfolios/{empty}")).await,
        StatusCode::OK,
    )
    .await;
    assert!(!empty_page.contains("holdings have no valuation"));
    assert!(!empty_page.contains("· Partial"));
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
}

#[tokio::test]
async fn valuation_completeness_is_separate_from_unknown_or_genuinely_zero_pnl() {
    for known in [false, true] {
        let fixture = TestApp::with_assets();
        let main = fixture
            .state
            .edit(move |store| {
                let main = store.data.portfolios[0].id;
                let asset = investment(store, "HOME");
                post(
                    store,
                    main,
                    asset,
                    TransactionKind::AssetIncrease,
                    100,
                    known.then_some(100),
                );
                Ok(main)
            })
            .await
            .unwrap();
        let before = std::fs::read(&fixture.path).unwrap();
        let list = html(fixture.get("/portfolios").await, StatusCode::OK).await;
        let main_row = row(&list, main);
        assert!(main_row.contains("Complete"));
        assert_eq!(main_row.contains("Unavailable"), !known);
        assert_eq!(main_row.contains("1 unavailable"), !known);
        for path in ["/".to_string(), format!("/portfolios/{main}")] {
            let page = html(fixture.get(&path).await, StatusCode::OK).await;
            let metric = page
                .split("<span class=\"metric-label\">Unrealized PnL</span>")
                .nth(1)
                .unwrap()
                .split("</div>")
                .next()
                .unwrap();
            assert!(metric.contains(if known { ">0.00</span>" } else { "Unavailable" }));
            assert!(!page.contains("holdings have no valuation"));
            assert_eq!(page.contains("No investment PnL can be calculated"), !known);
            assert_eq!(
                page.contains("investment holdings have unavailable PnL"),
                !known
            );
        }
        assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    }
}

#[tokio::test]
async fn a_known_zero_pnl_is_marked_partial_when_another_investment_is_unknown() {
    let fixture = TestApp::with_assets();
    let main = fixture
        .state
        .edit(|store| {
            let main = store.data.portfolios[0].id;
            let known = investment(store, "KNOWN");
            let unknown = investment(store, "UNKNOWN");
            post(
                store,
                main,
                known,
                TransactionKind::AssetIncrease,
                100,
                Some(100),
            );
            post(
                store,
                main,
                unknown,
                TransactionKind::AssetIncrease,
                100,
                None,
            );
            Ok(main)
        })
        .await
        .unwrap();
    for path in ["/".to_string(), format!("/portfolios/{main}")] {
        let page = html(fixture.get(&path).await, StatusCode::OK).await;
        assert!(page.contains("where cost basis is available · Partial"));
        assert!(page.contains("Partial PnL excludes these holdings"));
        assert!(page.contains("1 investment holdings have unavailable PnL"));
        assert!(!page.contains("No investment PnL can be calculated"));
        assert!(!page.contains("Net worth · Partial"));
    }
    let list = html(fixture.get("/portfolios").await, StatusCode::OK).await;
    let main_row = row(&list, main);
    assert!(main_row.contains("Complete"));
    assert!(main_row.contains("1 unavailable · Partial"));
    assert!(main_row.contains(">0.00</span>"));
}

#[tokio::test]
async fn negative_foreign_cash_keeps_its_balance_warning_after_valuation_is_available() {
    let fixture = TestApp::with_assets();
    fixture
        .state
        .edit(|store| {
            let usd = store.asset_by_symbol("USD").unwrap().id;
            post(
                store,
                store.data.portfolios[0].id,
                usd,
                TransactionKind::Withdraw,
                10,
                None,
            );
            Ok(())
        })
        .await
        .unwrap();
    let page = html(fixture.get("/").await, StatusCode::OK).await;
    assert!(page.contains("1 holdings have no valuation"));
    assert!(page.contains("1 non-liability holdings have negative balances"));
    fixture
        .state
        .edit(|store| add_manual_price(store, "USD", Decimal::new(9, 1), "EUR"))
        .await
        .unwrap();
    let page = html(fixture.get("/").await, StatusCode::OK).await;
    assert!(!page.contains("holdings have no valuation"));
    assert!(!page.contains("Net worth · Partial"));
    assert!(page.contains("1 non-liability holdings have negative balances"));
    assert!(page.contains(">-9.00</span>"));
}
