pub mod support;

use support::temp_store;
use tuifolio::{
    accounting::build_report,
    portfolios::{PortfolioInput, create_portfolio},
    store::Store,
};

fn create(store: &mut Store, name: &str) -> anyhow::Result<u64> {
    create_portfolio(store, PortfolioInput { name: name.into() })
}

#[test]
fn creates_empty_portfolios_with_stable_ids_and_trimmed_names() {
    let mut store = temp_store();
    let id = create(&mut store, "  Long-term investments  ").unwrap();
    assert_eq!(store.data.portfolios[0].id, id);
    assert_eq!(store.data.portfolios[0].name, "Long-term investments");
    assert!(store.data.assets.is_empty());
    assert!(store.data.transactions.is_empty());
    let report = build_report(&store.data);
    assert_eq!(report.portfolios[0].id, id);
    assert_eq!(report.portfolios[0].net_value, rust_decimal::Decimal::ZERO);
    store.save().unwrap();
    let path = store.path().clone();
    drop(store);
    let reopened = Store::open(Some(path)).unwrap();
    assert_eq!(reopened.data.portfolios[0].id, id);
}

#[test]
fn rejects_empty_long_and_duplicate_names_without_partial_changes() {
    let mut store = temp_store();
    create(&mut store, "Stocks").unwrap();
    let before = serde_json::to_value(&store.data).unwrap();
    for name in [
        "".to_string(),
        "   ".to_string(),
        " stocks ".to_string(),
        "x".repeat(201),
    ] {
        assert!(create(&mut store, &name).is_err());
        assert_eq!(serde_json::to_value(&store.data).unwrap(), before);
    }
}
