# Tuifolio

A local-first portfolio tracker with a dark, table-focused web interface.

Rust handles accounting and HTML rendering. Axum serves the app, Askama checks
templates at compile time, and HTMX updates navigation and forms. CSS, JavaScript,
and the font are embedded in the executable. No frontend build step is required.

## Start

Requires Rust 1.89 or newer.

```sh
cargo run -- import delta-exports
cargo run -- web
```

Open **http://127.0.0.1:3000**. Starting with `cargo run` also launches the web
interface. Press `Ctrl+C` to stop.

Choose a port or a separate datastore:

```sh
cargo run -- web --port 8080
cargo run -- --store /tmp/tuifolio-store.json web
```

The server binds to localhost. The interface and its assets work offline;
refreshing market prices requires an internet connection. Prices refresh when
you click **Refresh prices**.

## Web interface

- **Overview** shows net worth, assets, liabilities, unrealized PnL, and holdings.
  Filter holdings by portfolio or search by symbol and name. Missing valuations
  and negative non-liability balances are identified explicitly.
- **Portfolios** shows account totals and lets you create empty portfolios.
  Open a portfolio for its holdings and links to filtered transactions.
- **Transactions** has portfolio, asset, and text filters, with 50 entries per
  page. Asset scope can match the primary asset or any side, including quote
  currencies and fees. Each row's **Edit** link opens that transaction. Update
  its fields or use **Delete transaction** on the editor to remove it. Expand
  a row for its source, broker, exchange, and notes.
- **Add transaction** selects an existing portfolio by ID, with primary, quote,
  and fee assets selected from the asset directory. Asset choices show their
  symbol, name, and type, with IDs to distinguish duplicate symbols. Create an
  asset in **Assets** and a portfolio in **Portfolios** before using them in a
  transaction. Dates and times are entered in UTC. Validation errors preserve
  entries and selections.
- **Assets** combines the asset directory and pricing. Search by name, type, or
  provider symbol, refresh Yahoo prices, and record manual prices. Each asset has
  an editor for its display name, type, Yahoo symbol, TradingView symbol, and
  intrinsic valuation currency, alongside its recorded quotes and transaction
  links. Assets can be created before their first transaction.
- The **Base** selector switches between configured valuation currencies.
- Holdings link to their transactions, asset editor, and configured TradingView charts.

Forms and navigation also work with JavaScript disabled. Monetary amounts stay
as decimals in Rust; quantities retain their full precision. Fiat valuations
display two decimal places and crypto valuations display eight.

## Accounting and storage

The ledger-first JSON datastore lives in the platform's local data directory
under `tuifolio/store.json`, unless you supply `--store`.

- Tracks portfolios, assets, transactions, ledger entries, prices, and raw
  imported rows.
- Buys and sells explicitly control which sides post to ledger balances.
- Property and mortgage liabilities remain separate assets.
- Configured stock splits apply during import, including GME's 2022 4:1 split.
- Yahoo quotes support crypto, stocks, funds, fiat exchange rates, and
  commodities through per-asset provider symbols.
- PnL is derived from transactions, ledger entries, and prices. It is not
  persisted as canonical accounting truth.

Manual buys and sells require an asset quantity, an existing quote asset, and a
total quote amount. The quote amount supplies cost basis or proceeds for PnL.
Transactions reference portfolio and asset IDs and never create portfolios or
change asset definitions.
Fee assets and fee amounts must be supplied together.

Editing keeps the transaction ID and original source identifiers, then rebuilds
the ledger. The editor preserves existing asset and quote posting controls,
legacy incomplete values when unchanged, and timestamp precision on unchanged
dates. Deleting removes the transaction and its ledger effects, while retaining
asset definitions, portfolios, and prices. Failed saves roll back the edit or deletion.

Original imported rows remain recorded after edits and deletions. Reimporting
the same CSV therefore neither overwrites corrected entries nor restores deleted
ones. An explicit datastore rebuild starts again from the exports.

The transaction form's cash effect has two choices:

- **Cost basis only** records the quote amount for PnL without changing cash
  holdings. Use this when broker or exchange cash balances are not tracked.
- **Post cash movement** also posts the quote side. Buys reduce cash and sells
  increase it. Use this when cash deposits, withdrawals, and balances are tracked.

Fees always post to the ledger when supplied.

### Asset settings

Edits keep an asset's ID, so its transactions, ledger quantities, historical
prices, and stock splits keep their references. Local symbols are fixed after
creation; display names and provider mappings remain editable. User-edited
metadata takes precedence over imported metadata and automatic defaults,
including deliberately cleared fields.

Yahoo and TradingView symbols are entered explicitly. The editor checks their
format, but does not search or verify instruments against those providers.
Crypto Yahoo symbols must quote in USD, such as `BTC-USD`. TradingView symbols
include their exchange, such as `NASDAQ:AAPL` or `CRYPTO:BTCUSD`. Clear a Yahoo
symbol to use manual market prices; clear a TradingView symbol to remove the
chart link. Cash uses exchange rates independently of its Yahoo symbol field.

For property, liabilities, and custom assets, an intrinsic valuation currency
means each unit of quantity equals one unit of that currency. It takes
precedence over market prices. Leave the field blank to use recorded prices.

Currency assets used in accounting must retain a Cash or Crypto type. Their
display names and provider settings remain editable.

### Portfolios

Create portfolios from **Portfolios → Create portfolio** or the link in the
transaction form. Names are trimmed, limited to 200 characters, and checked for
duplicates without regard to ASCII letter case. Transactions select existing
portfolio IDs, including distinct legacy portfolios that share a name. Portfolio
creation does not create assets or transactions.

### Saving data

Saves atomically replace the JSON file. Web edits become active only after a
successful save. Price refreshes fetch outside the datastore lock, then merge
their quotes into the current data, preserving edits made during the refresh.
Quotes fetched with provider settings that changed during a refresh are discarded.

One Tuifolio process owns a datastore at a time. Stop the web server before
running CLI commands against the same datastore. The adjacent `.lock` file is
normal; the operating system releases its lock when the process exits.

## CLI commands

```sh
cargo run -- import delta-exports
cargo run -- rebuild delta-exports
cargo run -- sync-prices
cargo run -- summary
cargo run -- holding BTC
cargo run -- missing-prices
cargo run -- add-price GOLD 3000 EUR
cargo run -- base BTC
```

`rebuild` resets the datastore, including manual transactions and asset settings,
then imports the exports again. `base` selects a base currency and adds it to the
configured choices if necessary.

## Development

Web handlers and view models live in `src/web/`, HTML templates in `templates/`,
and embedded browser assets in `static/`. Rebuild Rust after changing templates
or assets. Vendored HTMX 2.0.8 and IBM Plex Sans have adjacent license files.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
