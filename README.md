# Tuifolio

Local-first Delta-style portfolio tracker for the terminal.

## MVP Features

- Imports Delta CSV exports from `delta-exports/`.
- Stores a ledger-first datastore as JSON at the platform data directory by default.
- Tracks portfolios, assets, transactions, ledger entries, prices, and raw imported rows.
- Models buys/sells as double-entry asset movements. Transactions explicitly control whether base and quote sides affect ledger balances.
- Supports fiat and crypto base currencies with fast switching in the TUI.
- Imports house/debt as asset plus liability instead of flattening it into one value.
- Provides Yahoo-based quote sync for crypto, stocks/funds, fiat FX, and commodities through per-asset Yahoo symbols, plus manual price entry for unsupported assets.
- Applies configured stock splits during import, including GME's 2022 4:1 split by default.
- Shows portfolio value, holdings, missing prices, negative balances, and derived PnL.

## Commands

```sh
cargo run -- import delta-exports
cargo run -- rebuild delta-exports
cargo run -- sync-prices
cargo run -- summary
cargo run -- holding BTC
cargo run -- missing-prices
cargo run -- tui
```

Use a custom datastore path when testing:

```sh
cargo run -- --store /tmp/tuifolio-store.json import delta-exports
```

Add a manual price for assets without a free provider:

```sh
cargo run -- add-price GOLD 3000 EUR
```

Switch or add a base currency:

```sh
cargo run -- base BTC
```

## TUI Keys

- `1`: Home
- `2`: Portfolios
- `3`: Holdings
- `b`: cycle configured base currencies
- `q`: quit

## Notes

PnL is derived from transactions, ledger entries, and prices. It is not persisted as canonical accounting truth.
