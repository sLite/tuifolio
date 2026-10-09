use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::{
    ledger::rebuild_ledger,
    model::{Id, StockSplit, StoreData},
    store::Store,
};

pub fn save_split(
    store: &mut Store,
    split: StockSplit,
    previous: Option<StockSplit>,
) -> anyhow::Result<()> {
    require_asset(&store.data, split.asset_id)?;
    validate_split(&split)?;
    let index = previous
        .as_ref()
        .map(|previous| find_split(&store.data, split.asset_id, previous))
        .transpose()?;
    validate_unique_date(&store.data, &split, index)?;
    let mut data = store.data.clone();
    if let Some(index) = index {
        data.config.stock_splits[index] = split;
    } else {
        data.config.stock_splits.push(split);
    }
    rebuild_ledger(&mut data)?;
    store.data = data;
    Ok(())
}

fn validate_unique_date(
    data: &StoreData,
    split: &StockSplit,
    index: Option<usize>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !data
            .config
            .stock_splits
            .iter()
            .enumerate()
            .any(|(i, other)| {
                Some(i) != index
                    && other.asset_id == split.asset_id
                    && other.effective_date == split.effective_date
            }),
        "a split already exists for this asset on that date"
    );
    Ok(())
}

pub fn delete_split(store: &mut Store, asset_id: Id, previous: StockSplit) -> anyhow::Result<()> {
    require_asset(&store.data, asset_id)?;
    let index = find_split(&store.data, asset_id, &previous)?;
    let mut data = store.data.clone();
    data.config.stock_splits.remove(index);
    rebuild_ledger(&mut data)?;
    store.data = data;
    Ok(())
}

fn require_asset(data: &StoreData, asset_id: Id) -> anyhow::Result<()> {
    anyhow::ensure!(
        data.assets.iter().any(|asset| asset.id == asset_id),
        "unknown asset"
    );
    Ok(())
}

fn find_split(data: &StoreData, asset_id: Id, previous: &StockSplit) -> anyhow::Result<usize> {
    anyhow::ensure!(
        previous.asset_id == asset_id,
        "split belongs to another asset"
    );
    // Splits have no persisted ID. Match the original values to reject stale edits.
    data.config
        .stock_splits
        .iter()
        .position(|split| split == previous)
        .ok_or_else(|| {
            anyhow::anyhow!("this split has changed or was deleted; reload the asset page")
        })
}

fn validate_split(split: &StockSplit) -> anyhow::Result<()> {
    let date = NaiveDate::parse_from_str(&split.effective_date, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("effective date must be a valid date in YYYY-MM-DD format"))?;
    anyhow::ensure!(
        date.format("%Y-%m-%d").to_string() == split.effective_date,
        "effective date must use YYYY-MM-DD format"
    );
    anyhow::ensure!(
        split.numerator > Decimal::ZERO,
        "new shares must be greater than zero"
    );
    anyhow::ensure!(
        split.denominator > Decimal::ZERO,
        "old shares must be greater than zero"
    );
    anyhow::ensure!(
        split.numerator != split.denominator,
        "a split must change the share quantity"
    );
    anyhow::ensure!(
        split
            .numerator
            .checked_div(split.denominator)
            .is_some_and(|ratio| ratio > Decimal::ZERO),
        "split ratio is outside the supported decimal range"
    );
    Ok(())
}
