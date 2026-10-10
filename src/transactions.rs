use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::{
    ledger::rebuild_ledger,
    model::{Id, LedgerEffect, StoreData, Transaction, TransactionKind},
    store::Store,
};

#[derive(Debug, Clone)]
pub struct ManualTransactionInput {
    pub portfolio_id: Id,
    pub timestamp: DateTime<Utc>,
    pub kind: TransactionKind,
    pub base_asset_id: Id,
    pub base_amount: Decimal,
    pub quote_asset_id: Option<Id>,
    pub quote_amount: Option<Decimal>,
    pub quote_ledger_effect: LedgerEffect,
    pub fee_asset_id: Option<Id>,
    pub fee_amount: Option<Decimal>,
    pub exchange: Option<String>,
    pub broker: Option<String>,
    pub notes: Option<String>,
}

impl From<&Transaction> for ManualTransactionInput {
    fn from(transaction: &Transaction) -> Self {
        Self {
            portfolio_id: transaction.portfolio_id,
            timestamp: transaction.timestamp,
            kind: transaction.kind,
            base_asset_id: transaction.base_asset_id,
            base_amount: transaction.base_amount,
            quote_asset_id: transaction.quote_asset_id,
            quote_amount: transaction.quote_amount,
            quote_ledger_effect: transaction.quote_ledger_effect,
            fee_asset_id: transaction.fee_asset_id,
            fee_amount: transaction.fee_amount,
            exchange: transaction.exchange.clone(),
            broker: transaction.broker.clone(),
            notes: transaction.notes.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManualTransactionResult {
    pub transaction_id: Id,
    pub portfolio_id: Id,
    pub asset_id: Id,
}

pub fn add_manual_transaction(
    store: &mut Store,
    input: ManualTransactionInput,
) -> anyhow::Result<ManualTransactionResult> {
    validate_input(&store.data, &input)?;
    let mut data = store.data.clone();
    let portfolio_id = input.portfolio_id;
    let transaction_id = data.allocate_id();
    let asset_id = input.base_asset_id;
    data.transactions
        .push(manual_transaction(transaction_id, portfolio_id, input));
    rebuild_ledger(&mut data)?;
    store.data = data;
    Ok(ManualTransactionResult {
        transaction_id,
        portfolio_id,
        asset_id,
    })
}

pub fn update_transaction(
    store: &mut Store,
    id: Id,
    input: ManualTransactionInput,
) -> anyhow::Result<ManualTransactionResult> {
    let index = transaction_index(&store.data, id)?;
    let previous = &store.data.transactions[index];
    validate_input(&store.data, &input)?;
    let mut transaction = manual_transaction(id, input.portfolio_id, input);
    transaction.source = previous.source.clone();
    transaction.source_row_hash = previous.source_row_hash.clone();
    let result = transaction_result(&transaction);
    let mut data = store.data.clone();
    data.transactions[index] = transaction;
    rebuild_ledger(&mut data)?;
    store.data = data;
    Ok(result)
}

pub fn delete_transaction(store: &mut Store, id: Id) -> anyhow::Result<ManualTransactionResult> {
    let index = transaction_index(&store.data, id)?;
    let result = transaction_result(&store.data.transactions[index]);
    let mut data = store.data.clone();
    data.transactions.remove(index);
    rebuild_ledger(&mut data)?;
    store.data = data;
    Ok(result)
}

fn transaction_index(data: &StoreData, id: Id) -> anyhow::Result<usize> {
    data.transactions
        .iter()
        .position(|transaction| transaction.id == id)
        .ok_or_else(|| anyhow::anyhow!("transaction does not exist"))
}

fn transaction_result(transaction: &Transaction) -> ManualTransactionResult {
    ManualTransactionResult {
        transaction_id: transaction.id,
        portfolio_id: transaction.portfolio_id,
        asset_id: transaction.base_asset_id,
    }
}

fn manual_transaction(id: Id, portfolio_id: Id, input: ManualTransactionInput) -> Transaction {
    Transaction {
        id,
        portfolio_id,
        timestamp: input.timestamp,
        kind: input.kind,
        base_asset_id: input.base_asset_id,
        base_amount: input.base_amount,
        quote_asset_id: input.quote_asset_id,
        quote_amount: input.quote_amount,
        quote_ledger_effect: if input.kind.has_implicit_zero_cost_basis()
            || input.kind == TransactionKind::StakingReward
        {
            LedgerEffect::Ignore
        } else {
            input.quote_ledger_effect
        },
        fee_asset_id: input.fee_asset_id,
        fee_amount: input.fee_amount,
        exchange: clean_string(input.exchange),
        broker: clean_string(input.broker),
        notes: clean_string(input.notes),
        source: "manual".into(),
        source_row_hash: format!("manual:{id}"),
    }
}

fn validate_input(data: &StoreData, input: &ManualTransactionInput) -> anyhow::Result<()> {
    anyhow::ensure!(
        data.portfolios
            .iter()
            .any(|portfolio| portfolio.id == input.portfolio_id),
        "the selected portfolio does not exist; choose an existing portfolio"
    );
    anyhow::ensure!(
        input.base_amount > Decimal::ZERO,
        "quantity must be greater than zero"
    );
    validate_asset_ids(data, input)?;
    if input.kind == TransactionKind::StakingReward {
        anyhow::ensure!(
            data.assets
                .iter()
                .any(|a| a.id == input.base_asset_id && a.kind == crate::model::AssetKind::Crypto),
            "staking rewards require a Crypto asset"
        );
        anyhow::ensure!(
            input.fee_asset_id != Some(input.base_asset_id)
                || input.fee_amount.is_none_or(|fee| fee < input.base_amount),
            "staking reward fee must be smaller than the received quantity"
        );
    }
    validate_quote(input)?;
    validate_fee(input)
}

fn validate_asset_ids(data: &StoreData, input: &ManualTransactionInput) -> anyhow::Result<()> {
    for (role, id) in [
        ("asset", Some(input.base_asset_id)),
        ("quote asset", input.quote_asset_id),
        ("fee asset", input.fee_asset_id),
    ] {
        if let Some(id) = id {
            anyhow::ensure!(
                data.assets.iter().any(|asset| asset.id == id),
                "the selected {role} does not exist; choose an existing asset"
            );
        }
    }
    Ok(())
}

fn validate_quote(input: &ManualTransactionInput) -> anyhow::Result<()> {
    if input.kind == TransactionKind::StakingReward {
        anyhow::ensure!(
            input.quote_amount.is_none(),
            "staking rewards use dated market prices, not a quote payment"
        );
        // Supported old deposits can retain a quote-asset metadata reference.
        return Ok(());
    }
    if !input.kind.supports_quote() {
        anyhow::ensure!(
            input.quote_asset_id.is_none() && input.quote_amount.is_none(),
            "quote fields are only applicable to buys, sells, asset increases, and asset decreases"
        );
        return Ok(());
    }
    if input.kind.requires_quote() {
        anyhow::ensure!(
            input.quote_asset_id.is_some(),
            "quote asset is required for buys and sells"
        );
        anyhow::ensure!(
            input.quote_amount.is_some(),
            "quote amount is required for buys and sells"
        );
    }
    if let Some(amount) = input.quote_amount {
        anyhow::ensure!(amount >= Decimal::ZERO, "quote amount cannot be negative");
        anyhow::ensure!(
            input.quote_asset_id.is_some(),
            "quote asset is required when quote amount is set"
        );
    }
    Ok(())
}

fn validate_fee(input: &ManualTransactionInput) -> anyhow::Result<()> {
    if let Some(amount) = input.fee_amount {
        anyhow::ensure!(
            amount > Decimal::ZERO,
            "fee amount must be greater than zero when set"
        );
        anyhow::ensure!(
            input.fee_asset_id.is_some(),
            "fee asset is required when fee amount is set"
        );
    }
    anyhow::ensure!(
        input.fee_asset_id.is_none() || input.fee_amount.is_some(),
        "fee amount is required when a fee asset is selected"
    );
    Ok(())
}

fn clean_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
