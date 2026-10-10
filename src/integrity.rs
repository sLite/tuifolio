use std::collections::HashSet;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::{
    currencies::normalize_reporting_currency,
    model::{Id, StoreData},
};

/// Structural checks, not new-transaction validation. Historical repairable
/// amounts and ignored quote metadata remain readable without being rewritten.
pub fn validate_store(data: &StoreData) -> anyhow::Result<()> {
    let mut all_ids = HashSet::new();
    for (role, ids) in [
        (
            "portfolio",
            data.portfolios.iter().map(|row| row.id).collect::<Vec<_>>(),
        ),
        ("asset", data.assets.iter().map(|row| row.id).collect()),
        (
            "transaction",
            data.transactions.iter().map(|row| row.id).collect(),
        ),
    ] {
        for id in ids {
            anyhow::ensure!(id > 0, "{role} ID must be positive");
            anyhow::ensure!(all_ids.insert(id), "duplicate datastore ID {id} on {role}");
        }
    }
    let largest = all_ids.iter().copied().max().unwrap_or(0);
    anyhow::ensure!(
        data.next_id > largest && data.next_id < Id::MAX,
        "next_id must exceed every existing ID and leave room for allocation"
    );
    let assets = data.assets.iter().map(|row| row.id).collect::<HashSet<_>>();
    let portfolios = data
        .portfolios
        .iter()
        .map(|row| row.id)
        .collect::<HashSet<_>>();
    let transactions = data
        .transactions
        .iter()
        .map(|row| row.id)
        .collect::<HashSet<_>>();
    for transaction in &data.transactions {
        anyhow::ensure!(
            portfolios.contains(&transaction.portfolio_id),
            "transaction {} references missing portfolio {}",
            transaction.id,
            transaction.portfolio_id
        );
        for (role, id) in [
            ("base", Some(transaction.base_asset_id)),
            ("quote", transaction.quote_asset_id),
            ("fee", transaction.fee_asset_id),
        ] {
            if let Some(id) = id {
                anyhow::ensure!(
                    assets.contains(&id),
                    "transaction {} references missing {role} asset {id}",
                    transaction.id
                );
            }
        }
    }
    for entry in &data.ledger_entries {
        anyhow::ensure!(
            assets.contains(&entry.asset_id),
            "ledger references missing asset {}",
            entry.asset_id
        );
        anyhow::ensure!(
            portfolios.contains(&entry.portfolio_id),
            "ledger references missing portfolio {}",
            entry.portfolio_id
        );
        anyhow::ensure!(
            transactions.contains(&entry.transaction_id),
            "ledger references missing transaction {}",
            entry.transaction_id
        );
    }
    for price in &data.prices {
        anyhow::ensure!(
            assets.contains(&price.asset_id),
            "price references missing asset {}",
            price.asset_id
        );
        anyhow::ensure!(!price.currency.trim().is_empty(), "price currency is empty");
        anyhow::ensure!(
            price.price >= Decimal::ZERO,
            "price for asset {} is negative",
            price.asset_id
        );
    }
    let mut split_keys = HashSet::new();
    for split in &data.config.stock_splits {
        anyhow::ensure!(
            assets.contains(&split.asset_id),
            "split references missing asset {}",
            split.asset_id
        );
        let date = NaiveDate::parse_from_str(&split.effective_date, "%Y-%m-%d")?;
        anyhow::ensure!(
            date.format("%Y-%m-%d").to_string() == split.effective_date,
            "split date must use YYYY-MM-DD"
        );
        anyhow::ensure!(
            split.numerator > Decimal::ZERO && split.denominator > Decimal::ZERO,
            "split numerator and denominator must be positive"
        );
        anyhow::ensure!(
            split_keys.insert((split.asset_id, date)),
            "duplicate split date for asset {}",
            split.asset_id
        );
    }
    anyhow::ensure!(
        !data.config.base_currencies.is_empty(),
        "reporting currency list is empty"
    );
    let mut currencies = HashSet::new();
    for code in &data.config.base_currencies {
        anyhow::ensure!(
            !code.trim().is_empty() && code == &code.trim().to_ascii_uppercase(),
            "reporting currency must be a nonempty canonical uppercase identifier"
        );
        normalize_reporting_currency(data, code)?;
        anyhow::ensure!(
            currencies.insert(code),
            "duplicate reporting currency {code}"
        );
    }
    for code in [
        &data.config.default_base_currency,
        &data.config.selected_base_currency,
    ] {
        anyhow::ensure!(
            currencies.contains(code),
            "reporting currency {code} is not configured"
        );
    }
    Ok(())
}
