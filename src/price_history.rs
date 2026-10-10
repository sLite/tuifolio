//! Daily price retention without changing existing transaction-time snapshots.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::model::{Id, StoreData};

type IndexedObservation = (DateTime<Utc>, usize);

/// Keep all observations today (and any future-dated records), the last
/// observation per completed UTC day and canonical asset/quote pair, and each
/// pair's latest observation at every existing transaction's UTC cutoff.
///
/// Preserving the entire price snapshot at each cutoff conservatively covers
/// intermediary FX, reciprocals, fees, closed positions and any reporting base.
/// Equal timestamps retain the first saved observation, matching accounting.
/// No source labels receive special treatment. Original records/order survive.
/// New backdated transactions may have less intraday history after compaction.
/// Returns the number of removed observations. Does not save the store.
pub fn compact_price_history(data: &mut StoreData, now: DateTime<Utc>) -> usize {
    let before = data.prices.len();
    let mut keep = vec![false; before];
    let mut pairs: BTreeMap<(Id, String), Vec<IndexedObservation>> = BTreeMap::new();
    for (index, price) in data.prices.iter().enumerate() {
        keep[index] = price.timestamp.date_naive() >= now.date_naive();
        pairs
            .entry((
                price.asset_id,
                crate::currencies::quote_unit(&price.currency).0,
            ))
            .or_default()
            .push((price.timestamp, index));
    }
    let mut cutoffs: Vec<_> = data.transactions.iter().map(|t| t.timestamp).collect();
    cutoffs.sort_unstable();
    cutoffs.dedup();
    for observations in pairs.values_mut() {
        observations.sort_unstable();
        // Index is the secondary sort key, so the first saved tie wins.
        observations.dedup_by_key(|(timestamp, _)| *timestamp);
        let mut daily = BTreeMap::new();
        for &(timestamp, index) in observations.iter() {
            daily.insert(timestamp.date_naive(), index);
        }
        for index in daily.into_values() {
            keep[index] = true;
        }
        for cutoff in &cutoffs {
            let end = observations.partition_point(|(timestamp, _)| timestamp <= cutoff);
            if end > 0 {
                keep[observations[end - 1].1] = true;
            }
        }
    }
    let mut index = 0;
    data.prices.retain(|_| {
        let retained = keep[index];
        index += 1;
        retained
    });
    before - data.prices.len()
}
