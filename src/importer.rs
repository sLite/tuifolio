use std::{
    collections::hash_map::DefaultHasher,
    fs::File,
    hash::{Hash, Hasher},
    path::Path,
};

use anyhow::Context;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    ledger::rebuild_ledger,
    model::{AssetKind, LedgerEffect, Transaction, TransactionKind, split_delta_asset},
    store::Store,
};

struct ImportedAsset {
    symbol: String,
    name: String,
    yahoo_symbol: Option<String>,
}

#[derive(Debug, Deserialize, Hash, serde::Serialize)]
struct DeltaRow {
    #[serde(rename = "Date")]
    date: String,
    #[serde(rename = "Way")]
    way: String,
    #[serde(rename = "Base amount")]
    base_amount: String,
    #[serde(rename = "Base currency (name)")]
    base_currency_name: String,
    #[serde(rename = "Base type")]
    base_type: String,
    #[serde(rename = "Quote amount")]
    quote_amount: Option<String>,
    #[serde(rename = "Quote currency")]
    quote_currency: Option<String>,
    #[serde(rename = "Exchange")]
    exchange: Option<String>,
    #[serde(rename = "Fee amount")]
    fee_amount: Option<String>,
    #[serde(rename = "Fee currency (name)")]
    fee_currency_name: Option<String>,
    #[serde(rename = "Broker")]
    broker: Option<String>,
    #[serde(rename = "Notes")]
    notes: Option<String>,
    #[serde(rename = "Sync Base Holding")]
    sync_base_holding: bool,
}

pub struct ImportSummary {
    pub imported: usize,
    pub skipped: usize,
}

pub fn import_delta_dir(store: &mut Store, dir: &Path) -> anyhow::Result<ImportSummary> {
    let mut imported = 0;
    let mut skipped = 0;
    let mut paths = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    for path in paths {
        if path.extension().and_then(|e| e.to_str()) != Some("csv") {
            continue;
        }
        let summary = import_delta_file(store, &path)?;
        imported += summary.imported;
        skipped += summary.skipped;
    }
    rebuild_ledger(&mut store.data)?;
    Ok(ImportSummary { imported, skipped })
}

pub fn import_delta_file(store: &mut Store, path: &Path) -> anyhow::Result<ImportSummary> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut reader = csv::Reader::from_reader(file);
    let portfolio_name = portfolio_name(path);
    let portfolio_id = store.portfolio_id(&portfolio_name);
    let mut imported = 0;
    let mut skipped = 0;

    for result in reader.deserialize() {
        let row: DeltaRow = result.with_context(|| format!("invalid row in {}", path.display()))?;
        let hash = row_hash(path, &row)?;
        if store.data.raw_rows.contains_key(&hash) {
            skipped += 1;
            continue;
        }
        let raw = serde_json::to_string(&row)?;
        import_row(
            store,
            portfolio_id,
            &path.display().to_string(),
            hash.clone(),
            row,
        )?;
        store.data.raw_rows.insert(hash, raw);
        imported += 1;
    }

    rebuild_ledger(&mut store.data)?;
    Ok(ImportSummary { imported, skipped })
}

fn import_row(
    store: &mut Store,
    portfolio_id: u64,
    source: &str,
    source_row_hash: String,
    row: DeltaRow,
) -> anyhow::Result<()> {
    let kind = TransactionKind::from_delta(&row.way)?;
    let timestamp = DateTime::parse_from_rfc3339(&row.date)?.with_timezone(&Utc);
    let base_amount = parse_decimal(&row.base_amount)?;
    let (base_symbol, base_name) = split_delta_asset(&row.base_currency_name);
    let base_asset = normalize_imported_asset(
        &base_symbol,
        &base_name,
        AssetKind::from_delta(&row.base_type),
    );
    let mut base_kind = AssetKind::from_delta(&row.base_type);
    if portfolio_is_house(source) && row.way == "WITHDRAW" {
        base_kind = AssetKind::Liability;
    }
    let posts_imported_fiat = imported_fiat_posts_to_ledger(source);
    let base_ledger_effect = ledger_effect_for_imported_asset(base_kind, posts_imported_fiat);
    let base_asset_id = store.asset_id_with_yahoo_symbol(
        &base_asset.symbol,
        &base_asset.name,
        base_kind,
        base_asset.yahoo_symbol,
    );

    let quote_amount = row
        .quote_amount
        .as_deref()
        .and_then(parse_optional_decimal)
        .transpose()?;
    let quote_asset_id = row
        .quote_currency
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|symbol| {
            let kind = quote_asset_kind(symbol);
            let asset = normalize_imported_asset(symbol, symbol, kind);
            store.asset_id_with_yahoo_symbol(&asset.symbol, &asset.name, kind, asset.yahoo_symbol)
        });
    let quote_ledger_effect = if row.sync_base_holding || is_sync_base_companion(&row) {
        LedgerEffect::Ignore
    } else {
        row.quote_currency
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|symbol| {
                ledger_effect_for_imported_asset(quote_asset_kind(symbol), posts_imported_fiat)
            })
            .unwrap_or_default()
    };
    let fee_amount = row
        .fee_amount
        .as_deref()
        .and_then(parse_optional_decimal)
        .transpose()?;
    let fee_asset_id = row
        .fee_currency_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|label| {
            let (symbol, name) = split_delta_asset(label);
            let kind = AssetKind::from_delta(&row.base_type);
            let asset = normalize_imported_asset(&symbol, &name, kind);
            store.asset_id_with_yahoo_symbol(&asset.symbol, &asset.name, kind, asset.yahoo_symbol)
        });
    let transaction_id = store.data.allocate_id();
    store.data.transactions.push(Transaction {
        id: transaction_id,
        portfolio_id,
        timestamp,
        kind,
        base_asset_id,
        base_amount,
        base_ledger_effect,
        quote_asset_id,
        quote_amount,
        quote_ledger_effect,
        fee_asset_id,
        fee_amount,
        exchange: clean(row.exchange),
        broker: clean(row.broker),
        notes: clean(row.notes),
        source: source.to_string(),
        source_row_hash,
    });
    Ok(())
}

fn portfolio_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("delta_"))
        .and_then(|s| s.split('_').next())
        .unwrap_or("Default")
        .to_string()
}

fn portfolio_is_house(source: &str) -> bool {
    source.contains("delta_House_")
}

fn imported_fiat_posts_to_ledger(source: &str) -> bool {
    source.contains("delta_Fiat_") || source.contains("delta_House_")
}

fn ledger_effect_for_imported_asset(kind: AssetKind, posts_imported_fiat: bool) -> LedgerEffect {
    if kind == AssetKind::Fiat && !posts_imported_fiat {
        LedgerEffect::Ignore
    } else {
        LedgerEffect::Post
    }
}

fn is_sync_base_companion(row: &DeltaRow) -> bool {
    row.notes
        .as_deref()
        .map(|notes| notes.starts_with("SYNC-BASE-HOLDINGS_"))
        .unwrap_or(false)
}

fn normalize_imported_asset(symbol: &str, name: &str, kind: AssetKind) -> ImportedAsset {
    ImportedAsset {
        symbol: symbol.to_string(),
        name: name.to_string(),
        yahoo_symbol: yahoo_symbol_for_imported_asset(symbol, kind),
    }
}

fn yahoo_symbol_for_imported_asset(symbol: &str, kind: AssetKind) -> Option<String> {
    match kind {
        AssetKind::Crypto if symbol == "IMX" => Some("IMX10603-USD".into()),
        AssetKind::Crypto if symbol == "UNI" => Some("UNI7083-USD".into()),
        AssetKind::Crypto => Some(format!("{symbol}-USD")),
        AssetKind::Stock | AssetKind::Fund | AssetKind::Commodity => match symbol {
            "GOLD" => Some("GC=F".into()),
            _ => Some(symbol.to_string()),
        },
        AssetKind::Fiat | AssetKind::Custom | AssetKind::Liability => None,
    }
}

fn row_hash(path: &Path, row: &DeltaRow) -> anyhow::Result<String> {
    let mut hasher = DefaultHasher::new();
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .hash(&mut hasher);
    row.hash(&mut hasher);
    Ok(format!("{:x}", hasher.finish()))
}

fn parse_decimal(value: &str) -> anyhow::Result<Decimal> {
    value
        .parse()
        .with_context(|| format!("invalid decimal: {value}"))
}

fn parse_optional_decimal(value: &str) -> Option<anyhow::Result<Decimal>> {
    if value.trim().is_empty() {
        None
    } else {
        Some(parse_decimal(value))
    }
}

fn clean(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

fn quote_asset_kind(symbol: &str) -> AssetKind {
    match symbol {
        "EUR" | "USD" | "CHF" | "GBP" => AssetKind::Fiat,
        _ => AssetKind::Crypto,
    }
}
