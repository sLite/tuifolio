use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::{
    ledger::rebuild_ledger,
    model::{
        Asset, AssetKind, Id, LedgerEffect, Portfolio, StoreData, Transaction, TransactionKind,
        default_tradingview_symbol,
    },
    store::Store,
};

#[derive(Debug, Clone)]
pub struct ManualTransactionInput {
    pub portfolio_name: String,
    pub timestamp: DateTime<Utc>,
    pub kind: TransactionKind,
    pub asset_symbol: String,
    pub asset_name: String,
    pub asset_kind: AssetKind,
    pub base_amount: Decimal,
    pub quote_symbol: Option<String>,
    pub quote_amount: Option<Decimal>,
    pub quote_ledger_effect: LedgerEffect,
    pub fee_symbol: Option<String>,
    pub fee_amount: Option<Decimal>,
    pub exchange: Option<String>,
    pub broker: Option<String>,
    pub notes: Option<String>,
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
    validate_input(&input)?;
    let mut data = store.data.clone();
    let portfolio_id = portfolio_id(&mut data, input.portfolio_name.trim());
    let asset_symbol = normalize_symbol(&input.asset_symbol);
    reject_mismatched_existing_asset(&data, &asset_symbol, input.asset_kind)?;
    let asset_name = clean_string(&input.asset_name).unwrap_or_else(|| asset_symbol.clone());
    let asset_id = asset_id_with_metadata(
        &mut data,
        &asset_symbol,
        &asset_name,
        input.asset_kind,
        default_yahoo_symbol(&asset_symbol, input.asset_kind),
        default_tradingview_symbol(&asset_symbol, input.asset_kind),
        None,
    );
    let quote_asset_id = input
        .quote_symbol
        .as_deref()
        .and_then(clean_str)
        .map(|symbol| resolve_currency_asset(&mut data, symbol));
    let fee_asset_id = input
        .fee_symbol
        .as_deref()
        .and_then(clean_str)
        .map(|symbol| resolve_currency_asset(&mut data, symbol));

    let transaction_id = data.allocate_id();
    data.transactions.push(Transaction {
        id: transaction_id,
        portfolio_id,
        timestamp: input.timestamp,
        kind: input.kind,
        base_asset_id: asset_id,
        base_amount: input.base_amount,
        base_ledger_effect: LedgerEffect::Post,
        quote_asset_id,
        quote_amount: input.quote_amount,
        quote_ledger_effect: input.quote_ledger_effect,
        fee_asset_id,
        fee_amount: input.fee_amount,
        exchange: clean_string_opt(input.exchange),
        broker: clean_string_opt(input.broker),
        notes: clean_string_opt(input.notes),
        source: "manual".into(),
        source_row_hash: format!("manual:{transaction_id}"),
    });
    rebuild_ledger(&mut data)?;
    store.data = data;
    Ok(ManualTransactionResult {
        transaction_id,
        portfolio_id,
        asset_id,
    })
}

fn validate_input(input: &ManualTransactionInput) -> anyhow::Result<()> {
    anyhow::ensure!(
        !input.portfolio_name.trim().is_empty(),
        "portfolio is required"
    );
    anyhow::ensure!(
        !input.asset_symbol.trim().is_empty(),
        "asset symbol is required"
    );
    anyhow::ensure!(
        input.base_amount > Decimal::ZERO,
        "quantity must be greater than zero"
    );
    if matches!(input.kind, TransactionKind::Buy | TransactionKind::Sell) {
        anyhow::ensure!(
            input.quote_symbol.as_deref().and_then(clean_str).is_some(),
            "quote currency is required for buys and sells"
        );
        anyhow::ensure!(
            input
                .quote_amount
                .is_some_and(|amount| amount > Decimal::ZERO),
            "quote amount must be greater than zero for buys and sells"
        );
    }
    if input
        .fee_amount
        .is_some_and(|amount| amount <= Decimal::ZERO)
    {
        anyhow::bail!("fee amount must be greater than zero when set");
    }
    if input.fee_amount.is_some() {
        anyhow::ensure!(
            input.fee_symbol.as_deref().and_then(clean_str).is_some(),
            "fee currency is required when fee amount is set"
        );
    }
    Ok(())
}

fn reject_mismatched_existing_asset(
    data: &StoreData,
    symbol: &str,
    kind: AssetKind,
) -> anyhow::Result<()> {
    let Some(existing) = data.assets.iter().find(|asset| asset.symbol == symbol) else {
        return Ok(());
    };
    anyhow::ensure!(
        existing.kind == kind,
        "asset {symbol} already exists as {:?}; select that kind instead of {:?}",
        existing.kind,
        kind
    );
    Ok(())
}

fn portfolio_id(data: &mut StoreData, name: &str) -> Id {
    if let Some(portfolio) = data.portfolios.iter().find(|p| p.name == name) {
        return portfolio.id;
    }
    let id = data.allocate_id();
    data.portfolios.push(Portfolio {
        id,
        name: name.to_string(),
    });
    id
}

fn asset_id_with_metadata(
    data: &mut StoreData,
    symbol: &str,
    name: &str,
    kind: AssetKind,
    yahoo_symbol: Option<String>,
    tradingview_symbol: Option<String>,
    valuation_currency: Option<String>,
) -> Id {
    if let Some(asset) = data
        .assets
        .iter_mut()
        .find(|asset| asset.symbol == symbol && asset.kind == kind)
    {
        if asset.yahoo_symbol.is_none() && yahoo_symbol.is_some() {
            asset.yahoo_symbol = yahoo_symbol;
        }
        if asset.tradingview_symbol.is_none() && tradingview_symbol.is_some() {
            asset.tradingview_symbol = tradingview_symbol;
        }
        if asset.valuation_currency.is_none() && valuation_currency.is_some() {
            asset.valuation_currency = valuation_currency;
        }
        return asset.id;
    }
    let id = data.allocate_id();
    data.assets.push(Asset {
        id,
        symbol: symbol.to_string(),
        name: name.to_string(),
        kind,
        yahoo_symbol,
        tradingview_symbol,
        valuation_currency,
    });
    id
}

fn resolve_currency_asset(data: &mut StoreData, symbol: &str) -> Id {
    let symbol = normalize_symbol(symbol);
    let kind = currency_asset_kind(&symbol);
    let yahoo_symbol = default_yahoo_symbol(&symbol, kind);
    asset_id_with_metadata(data, &symbol, &symbol, kind, yahoo_symbol, None, None)
}

fn currency_asset_kind(symbol: &str) -> AssetKind {
    match symbol {
        "EUR" | "USD" | "CHF" | "GBP" => AssetKind::Fiat,
        _ => AssetKind::Crypto,
    }
}

fn default_yahoo_symbol(symbol: &str, kind: AssetKind) -> Option<String> {
    match kind {
        AssetKind::Crypto if symbol == "IMX" => Some("IMX10603-USD".into()),
        AssetKind::Crypto if symbol == "UNI" => Some("UNI7083-USD".into()),
        AssetKind::Crypto => Some(format!("{symbol}-USD")),
        AssetKind::Stock | AssetKind::Fund | AssetKind::Commodity => match symbol {
            "GOLD" => Some("GC=F".into()),
            _ => Some(symbol.to_string()),
        },
        AssetKind::Fiat | AssetKind::Custom | AssetKind::Property | AssetKind::Liability => None,
    }
}

fn normalize_symbol(symbol: &str) -> String {
    symbol.trim().to_ascii_uppercase()
}

fn clean_string(value: &str) -> Option<String> {
    clean_str(value).map(ToOwned::to_owned)
}

fn clean_string_opt(value: Option<String>) -> Option<String> {
    value.and_then(|value| clean_string(&value))
}

fn clean_str(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() { None } else { Some(value) }
}
