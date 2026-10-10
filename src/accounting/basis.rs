//! Derived weighted-average accounting. Raw transactions and quantity postings
//! remain authoritative; none of these calculations are saved to the store.
use std::collections::{BTreeMap, HashMap};

use rust_decimal::Decimal;

use crate::model::{Asset, AssetKind, Id, LedgerRole, StoreData, Transaction, TransactionKind};

#[derive(Debug, Clone)]
pub(super) struct Position {
    pub quantity: Decimal,
    pub basis: Option<Decimal>,
    pub realized: Option<Decimal>,
    pub disposed: bool,
}

impl Default for Position {
    fn default() -> Self {
        Self {
            quantity: Decimal::ZERO,
            basis: Some(Decimal::ZERO),
            realized: Some(Decimal::ZERO),
            disposed: false,
        }
    }
}

impl Position {
    fn acquire(&mut self, amount: Decimal, cost: Option<Decimal>) {
        // A closed position starts a fresh basis pool, not the previous trade's profit.
        if self.quantity.is_zero() {
            self.basis = Some(Decimal::ZERO);
        }
        self.basis = self.basis.zip(cost).and_then(|(a, b)| a.checked_add(b));
        match self.quantity.checked_add(amount) {
            Some(q) if q >= Decimal::ZERO => self.quantity = q,
            _ => {
                self.basis = None;
                self.quantity = self.quantity.saturating_add(amount);
            }
        }
    }

    fn dispose(&mut self, amount: Decimal, proceeds: Option<Decimal>, realize: bool) {
        let allocated = if amount == self.quantity && amount > Decimal::ZERO {
            self.basis
        } else if amount > Decimal::ZERO && amount < self.quantity {
            self.basis
                .and_then(|basis| basis.checked_mul(amount)?.checked_div(self.quantity))
        } else {
            None
        };
        self.basis = self
            .basis
            .zip(allocated)
            .and_then(|(basis, removed)| basis.checked_sub(removed));
        self.quantity = self.quantity.saturating_sub(amount);
        if self.quantity.is_zero() {
            self.basis = Some(Decimal::ZERO);
        }
        if self.quantity < Decimal::ZERO {
            self.basis = None;
        }
        if realize {
            self.disposed = true;
            self.realized = self.realized.zip(proceeds.zip(allocated)).and_then(
                |(previous, (received, cost))| previous.checked_add(received.checked_sub(cost)?),
            );
        }
    }

    fn revalue(&mut self, delta: Decimal) {
        if self.quantity.is_zero() && delta > Decimal::ZERO {
            self.basis = None;
        }
        self.quantity = self.quantity.saturating_add(delta);
    }
}

fn valued_amount(
    data: &StoreData,
    t: &Transaction,
    asset: &Asset,
    amount: Decimal,
    base: &str,
    assets: &HashMap<Id, &Asset>,
) -> Option<Decimal> {
    if amount.is_zero() {
        return Some(Decimal::ZERO);
    }
    super::value_in_base(
        data,
        asset,
        amount,
        base,
        &super::prices_at(data, t.timestamp),
        assets,
    )
}

fn amount(data: &StoreData, t: &Transaction, asset_id: Id, raw: Decimal) -> Option<Decimal> {
    crate::ledger::split_adjusted_amount(data, asset_id, t.timestamp, raw).ok()
}

fn currency(data: &StoreData, asset: &Asset) -> bool {
    crate::currencies::currency_asset(data, &asset.symbol).is_some_and(|a| a.id == asset.id)
}

// Choose consideration by economic leg, not whether the user wrote Buy B/pay A
// or Sell A/receive B. Cash takes precedence. Crypto/crypto exchanges use the
// disposed asset's recorded transaction-time value. No current-price fallback.
fn trade_value(
    data: &StoreData,
    t: &Transaction,
    received: (&Asset, Decimal),
    disposed: (&Asset, Decimal),
    base: &str,
    assets: &HashMap<Id, &Asset>,
) -> Option<Decimal> {
    if t.quote_amount.is_some_and(|q| q.is_zero()) {
        return Some(Decimal::ZERO);
    }
    let (r, rq) = received;
    let (d, dq) = disposed;
    let chosen = if super::currency_identity(data, r, base) {
        (r, rq)
    } else if super::currency_identity(data, d, base) {
        (d, dq)
    } else if r.kind == AssetKind::Fiat {
        (r, rq)
    } else if d.kind == AssetKind::Fiat {
        (d, dq)
    } else if currency(data, r) && !currency(data, d) {
        (r, rq)
    } else {
        (d, dq)
    };
    valued_amount(data, t, chosen.0, chosen.1, base, assets)
}

fn investment(asset: &Asset) -> bool {
    !matches!(asset.kind, AssetKind::Fiat | AssetKind::Liability)
}

pub(super) fn replay(
    data: &StoreData,
    base: &str,
    assets: &HashMap<Id, &Asset>,
) -> BTreeMap<(Id, Id), Position> {
    let mut pools: BTreeMap<(Id, Id), Position> = BTreeMap::new();
    let mut entries: HashMap<Id, Vec<_>> = HashMap::new();
    for e in &data.ledger_entries {
        entries.entry(e.transaction_id).or_default().push(e);
    }
    let mut transactions: Vec<_> = data.transactions.iter().collect();
    transactions.sort_by_key(|t| (t.timestamp, t.id));
    for t in transactions {
        let Some(primary) = assets.get(&t.base_asset_id).copied() else {
            continue;
        };
        let legs = entries.get(&t.id).cloned().unwrap_or_default();
        let primary_delta = legs
            .iter()
            .find(|e| matches!(e.role, LedgerRole::Base))
            .map(|e| e.quantity_delta)
            .unwrap_or(Decimal::ZERO);
        let quote_delta = legs
            .iter()
            .find(|e| matches!(e.role, LedgerRole::Quote))
            .map(|e| e.quantity_delta);
        let fee_leg = legs.iter().find(|e| matches!(e.role, LedgerRole::Fee));
        let fee = fee_leg.and_then(|e| {
            assets
                .get(&e.asset_id)
                .copied()
                .map(|asset| (asset, -e.quantity_delta))
        });
        let quote = t
            .quote_asset_id
            .zip(t.quote_amount)
            .and_then(|(id, q)| assets.get(&id).copied().zip(amount(data, t, id, q)));
        let trade = t.kind.supports_quote() && quote.is_some();
        let incoming = primary_delta > Decimal::ZERO;
        let received = if incoming {
            Some((primary, primary_delta))
        } else {
            quote
        };
        let disposed = if incoming {
            quote
        } else {
            Some((primary, -primary_delta))
        };
        let received_posted = incoming || quote_delta.is_some();
        let disposed_posted = !incoming || quote_delta.is_some();
        let mut fee_handled = false;

        if trade && let (Some(received), Some(disposed)) = (received, disposed) {
            let gross = trade_value(data, t, received, disposed, base, assets);
            let disposal_fee = !received_posted
                || received.0.kind == AssetKind::Fiat
                || (disposed.0.kind != AssetKind::Fiat
                    && !currency(data, disposed.0)
                    && currency(data, received.0));
            let embedded_fee =
                !disposal_fee && received_posted && fee.is_some_and(|(a, _)| a.id == received.0.id);
            let fee_value = if let Some((asset, q)) = fee {
                if embedded_fee {
                    Some(Decimal::ZERO)
                } else {
                    valued_amount(data, t, asset, q, base, assets)
                }
            } else {
                Some(Decimal::ZERO)
            };
            let cost = if disposal_fee {
                gross.zip(fee_value).and_then(|(g, f)| g.checked_sub(f))
            } else {
                gross.zip(fee_value).and_then(|(g, f)| g.checked_add(f))
            };
            let proceeds = if disposal_fee { cost } else { gross };
            if disposed_posted && investment(disposed.0) {
                pools
                    .entry((t.portfolio_id, disposed.0.id))
                    .or_default()
                    .dispose(disposed.1, proceeds, true);
            }
            if received_posted && investment(received.0) {
                let q = if embedded_fee {
                    received.1.checked_sub(fee.unwrap().1)
                } else {
                    Some(received.1)
                };
                if let Some(q) = q.filter(|q| *q >= Decimal::ZERO) {
                    pools
                        .entry((t.portfolio_id, received.0.id))
                        .or_default()
                        .acquire(q, cost);
                } else {
                    let pool = pools.entry((t.portfolio_id, received.0.id)).or_default();
                    pool.acquire(received.1, None);
                }
            }
            fee_handled = embedded_fee;
        } else if investment(primary) {
            let pool = pools.entry((t.portfolio_id, primary.id)).or_default();
            if t.kind == TransactionKind::Gift {
                let embedded = fee.is_some_and(|(asset, _)| asset.id == primary.id);
                let q = if embedded {
                    primary_delta.checked_sub(fee.unwrap().1)
                } else {
                    Some(primary_delta)
                };
                let cost = if let Some((asset, q)) = fee.filter(|_| !embedded) {
                    valued_amount(data, t, asset, q, base, assets)
                } else {
                    Some(Decimal::ZERO)
                };
                pool.acquire(q.unwrap_or(primary_delta), cost);
                fee_handled = embedded;
            } else if primary.valuation_currency.is_some()
                && matches!(
                    t.kind,
                    TransactionKind::AssetIncrease | TransactionKind::AssetDecrease
                )
            {
                pool.revalue(primary_delta);
            } else if primary_delta > Decimal::ZERO {
                pool.acquire(primary_delta, None);
            } else if primary_delta < Decimal::ZERO {
                pool.dispose(-primary_delta, None, false);
            }
        }
        if !fee_handled
            && let Some((asset, q)) = fee
            && investment(asset)
        {
            let value = valued_amount(data, t, asset, q, base, assets);
            pools
                .entry((t.portfolio_id, asset.id))
                .or_default()
                .dispose(q, value, true);
        }
    }
    pools
}
