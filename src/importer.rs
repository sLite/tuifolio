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
    model::{AssetKind, LedgerEffect, StockSplit, Transaction, TransactionKind},
    store::Store,
};

const PROPERTY_PURCHASE_DATE: &str = "2013-01-01T00:00:00.000Z";
const PROPERTY_PURCHASE_PRICE_EUR: Decimal = Decimal::from_parts(255000, 0, 0, false, 0);
const MORTGAGE_START_DATE: &str = "2013-01-01T00:00:00.000Z";
const MORTGAGE_CURRENT_DATE: &str = "2026-05-30T00:00:00.000Z";
const MORTGAGE_START_AMOUNT_EUR: Decimal = Decimal::from_parts(255000, 0, 0, false, 0);
const MORTGAGE_CURRENT_AMOUNT_EUR: Decimal = Decimal::from_parts(110000, 0, 0, false, 0);
const GME_SPLIT_DATE: &str = "2022-07-22";

struct ImportedAsset {
    symbol: String,
    name: String,
    yahoo_symbol: Option<String>,
    valuation_currency: Option<String>,
}

struct TransactionDraft {
    portfolio_id: u64,
    timestamp: DateTime<Utc>,
    kind: TransactionKind,
    base_asset_id: u64,
    base_amount: Decimal,
    source: String,
    source_row_hash: String,
    notes: Option<String>,
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
    ensure_gme_stock_split(store);
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

    ensure_gme_stock_split(store);
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
    let house_loan = is_house_loan(source, &row);
    if is_house_property(source, &row) {
        return import_house_property(store, portfolio_id, source, source_row_hash, row);
    }
    if house_loan {
        return import_house_loan(store, portfolio_id, source, source_row_hash);
    }
    let kind = transaction_kind_from_delta(&row.way)?;
    let timestamp = DateTime::parse_from_rfc3339(&row.date)?.with_timezone(&Utc);
    let base_amount = parse_decimal(&row.base_amount)?;
    let (base_symbol, base_name) = split_delta_asset(&row.base_currency_name);
    let base_kind = asset_kind_from_delta(&row.base_type);
    let base_asset = normalize_imported_asset(&base_symbol, &base_name, base_kind, house_loan);
    let base_ledger_effect = base_ledger_effect_for_imported_asset(base_kind, kind, source);
    let base_asset_id = store.asset_id_with_metadata(
        &base_asset.symbol,
        &base_asset.name,
        base_kind,
        base_asset.yahoo_symbol,
        base_asset.valuation_currency,
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
            let asset = normalize_imported_asset(symbol, symbol, kind, false);
            store.asset_id_with_metadata(
                &asset.symbol,
                &asset.name,
                kind,
                asset.yahoo_symbol,
                asset.valuation_currency,
            )
        });
    let quote_ledger_effect = if row.sync_base_holding || is_sync_base_companion(&row) {
        LedgerEffect::Ignore
    } else {
        row.quote_currency
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|symbol| quote_ledger_effect_for_imported_asset(quote_asset_kind(symbol), source))
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
            let kind = asset_kind_from_delta(&row.base_type);
            let asset = normalize_imported_asset(&symbol, &name, kind, false);
            store.asset_id_with_metadata(
                &asset.symbol,
                &asset.name,
                kind,
                asset.yahoo_symbol,
                asset.valuation_currency,
            )
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
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("delta_"))
        .and_then(|s| s.split('_').next())
        .unwrap_or("Default")
        .to_string();
    if name == "House" {
        "Hintersdorf".into()
    } else {
        name
    }
}

fn portfolio_is_house(source: &str) -> bool {
    source.contains("delta_House_")
}

fn is_house_loan(source: &str, row: &DeltaRow) -> bool {
    portfolio_is_house(source) && row.way == "WITHDRAW"
}

fn is_house_property(source: &str, row: &DeltaRow) -> bool {
    portfolio_is_house(source) && row.way == "DEPOSIT"
}

fn imported_quote_fiat_posts_to_ledger(source: &str) -> bool {
    source.contains("delta_Fiat_") || source.contains("delta_House_")
}

fn base_ledger_effect_for_imported_asset(
    kind: AssetKind,
    transaction_kind: TransactionKind,
    source: &str,
) -> LedgerEffect {
    if kind == AssetKind::Fiat
        && !matches!(
            transaction_kind,
            TransactionKind::Deposit | TransactionKind::Withdraw
        )
        && !imported_quote_fiat_posts_to_ledger(source)
    {
        LedgerEffect::Ignore
    } else {
        LedgerEffect::Post
    }
}

fn quote_ledger_effect_for_imported_asset(kind: AssetKind, source: &str) -> LedgerEffect {
    if kind == AssetKind::Fiat && !imported_quote_fiat_posts_to_ledger(source) {
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

fn normalize_imported_asset(
    symbol: &str,
    name: &str,
    kind: AssetKind,
    house_loan: bool,
) -> ImportedAsset {
    if house_loan {
        return ImportedAsset {
            symbol: "MORTGAGE".into(),
            name: "Kredit".into(),
            yahoo_symbol: None,
            valuation_currency: Some(symbol.to_string()),
        };
    }
    ImportedAsset {
        symbol: symbol.to_string(),
        name: name.to_string(),
        yahoo_symbol: yahoo_symbol_for_imported_asset(symbol, kind),
        valuation_currency: None,
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
        AssetKind::Fiat | AssetKind::Custom | AssetKind::Property | AssetKind::Liability => None,
    }
}

fn import_house_property(
    store: &mut Store,
    portfolio_id: u64,
    source: &str,
    source_row_hash: String,
    row: DeltaRow,
) -> anyhow::Result<()> {
    let timestamp = DateTime::parse_from_rfc3339(&row.date)?.with_timezone(&Utc);
    let current_value = parse_decimal(&row.base_amount)?;
    let (currency, _) = split_delta_asset(&row.base_currency_name);
    let property_asset_id = store.asset_id_with_metadata(
        "PROPERTY",
        "Grundstück",
        AssetKind::Property,
        None,
        Some("EUR".into()),
    );

    push_transaction(
        store,
        TransactionDraft {
            portfolio_id,
            timestamp: DateTime::parse_from_rfc3339(PROPERTY_PURCHASE_DATE)?.with_timezone(&Utc),
            kind: TransactionKind::AssetIncrease,
            base_asset_id: property_asset_id,
            base_amount: PROPERTY_PURCHASE_PRICE_EUR,
            source: source.to_string(),
            source_row_hash: format!("{source_row_hash}:purchase"),
            notes: Some(format!("Purchase price ({currency})")),
        },
    );

    let valuation_increase = current_value - PROPERTY_PURCHASE_PRICE_EUR;
    if !valuation_increase.is_zero() {
        let kind = if valuation_increase > Decimal::ZERO {
            TransactionKind::AssetIncrease
        } else {
            TransactionKind::AssetDecrease
        };
        push_transaction(
            store,
            TransactionDraft {
                portfolio_id,
                timestamp,
                kind,
                base_asset_id: property_asset_id,
                base_amount: valuation_increase.abs(),
                source: source.to_string(),
                source_row_hash,
                notes: clean(row.notes),
            },
        );
    }

    Ok(())
}

fn import_house_loan(
    store: &mut Store,
    portfolio_id: u64,
    source: &str,
    source_row_hash: String,
) -> anyhow::Result<()> {
    let mortgage_asset_id = store.asset_id_with_metadata(
        "MORTGAGE",
        "Kredit",
        AssetKind::Liability,
        None,
        Some("EUR".into()),
    );
    push_transaction(
        store,
        TransactionDraft {
            portfolio_id,
            timestamp: DateTime::parse_from_rfc3339(MORTGAGE_START_DATE)?.with_timezone(&Utc),
            kind: TransactionKind::LiabilityIncrease,
            base_asset_id: mortgage_asset_id,
            base_amount: MORTGAGE_START_AMOUNT_EUR,
            source: source.to_string(),
            source_row_hash: format!("{source_row_hash}:start"),
            notes: Some("Initial mortgage".into()),
        },
    );
    let reduction = MORTGAGE_START_AMOUNT_EUR - MORTGAGE_CURRENT_AMOUNT_EUR;
    if !reduction.is_zero() {
        push_transaction(
            store,
            TransactionDraft {
                portfolio_id,
                timestamp: DateTime::parse_from_rfc3339(MORTGAGE_CURRENT_DATE)?.with_timezone(&Utc),
                kind: TransactionKind::LiabilityDecrease,
                base_asset_id: mortgage_asset_id,
                base_amount: reduction,
                source: source.to_string(),
                source_row_hash,
                notes: Some("Current mortgage balance".into()),
            },
        );
    }
    Ok(())
}

fn ensure_gme_stock_split(store: &mut Store) {
    let Some(asset) = store
        .data
        .assets
        .iter()
        .find(|asset| asset.symbol == "GME" && asset.kind == AssetKind::Stock)
    else {
        return;
    };
    if store
        .data
        .config
        .stock_splits
        .iter()
        .any(|split| split.asset_id == asset.id && split.effective_date == GME_SPLIT_DATE)
    {
        return;
    }
    store.data.config.stock_splits.push(StockSplit {
        asset_id: asset.id,
        legacy_symbol: None,
        effective_date: GME_SPLIT_DATE.into(),
        numerator: Decimal::new(4, 0),
        denominator: Decimal::ONE,
    });
}

fn push_transaction(store: &mut Store, draft: TransactionDraft) {
    let transaction_id = store.data.allocate_id();
    store.data.transactions.push(Transaction {
        id: transaction_id,
        portfolio_id: draft.portfolio_id,
        timestamp: draft.timestamp,
        kind: draft.kind,
        base_asset_id: draft.base_asset_id,
        base_amount: draft.base_amount,
        base_ledger_effect: LedgerEffect::Post,
        quote_asset_id: None,
        quote_amount: None,
        quote_ledger_effect: LedgerEffect::Post,
        fee_asset_id: None,
        fee_amount: None,
        exchange: None,
        broker: None,
        notes: draft.notes,
        source: draft.source,
        source_row_hash: draft.source_row_hash,
    });
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

fn asset_kind_from_delta(value: &str) -> AssetKind {
    match value {
        "FIAT" => AssetKind::Fiat,
        "CRYPTO" => AssetKind::Crypto,
        "STOCK" => AssetKind::Stock,
        "FUND" => AssetKind::Fund,
        "COMMODITY" => AssetKind::Commodity,
        _ => AssetKind::Custom,
    }
}

fn transaction_kind_from_delta(value: &str) -> anyhow::Result<TransactionKind> {
    match value {
        "BUY" => Ok(TransactionKind::Buy),
        "SELL" => Ok(TransactionKind::Sell),
        "DEPOSIT" => Ok(TransactionKind::Deposit),
        "WITHDRAW" => Ok(TransactionKind::Withdraw),
        other => anyhow::bail!("unsupported Delta transaction kind: {other}"),
    }
}

pub fn split_delta_asset(label: &str) -> (String, String) {
    let trimmed = label.trim();
    if let Some((symbol, rest)) = trimmed.split_once(" (") {
        let name = rest.strip_suffix(')').unwrap_or(rest);
        (
            symbol.trim().trim_end_matches('*').to_string(),
            name.to_string(),
        )
    } else {
        (trimmed.to_string(), trimmed.to_string())
    }
}
