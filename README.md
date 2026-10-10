# Tuifolio

A local-first portfolio tracker with a dark, table-focused web interface.

Rust handles accounting and HTML rendering. Axum serves the app, Askama checks
templates at compile time, and HTMX updates navigation and forms. CSS, JavaScript,
and the font are embedded in the executable. No frontend build step is required.

## Start

Requires Rust 1.89 or newer.

```sh
cargo run -- web
```

Open **http://127.0.0.1:3000**. Starting with `cargo run` also launches the web
interface. Press `Ctrl+C` to stop.

Create a portfolio and your assets in the web interface, then add transactions.
CSV import and CSV-based datastore rebuilding have been removed.

Choose a port or a separate datastore:

```sh
cargo run -- web --port 8080
cargo run -- --store /tmp/tuifolio-store.json web
```

The server binds to localhost. The interface and its assets work offline;
refreshing market prices requires an internet connection.

### Automatic price refresh

While the web server runs, it refreshes prices in the background at startup and
then every 5 minutes. The interval starts after each refresh attempt finishes,
so slow requests do not overlap or produce catch-up bursts. Manual **Refresh
prices** uses the same refresh slot; a scheduled attempt skips if another
refresh is already running.

Change the interval in seconds, or use `0` for manual-only refreshes:

```sh
cargo run -- web --price-refresh-seconds 600
cargo run -- web --price-refresh-seconds 0
```

Failures retain existing prices and retry after the configured interval. Browsing
and transaction edits remain available during refreshes. New quotes appear when
you navigate or reload a view. On shutdown, future scheduled attempts stop and
an active refresh finishes before the server exits. Refresh scheduling is a
runtime option; it adds no datastore fields.

## Web interface

- **Overview** shows net worth, assets, liabilities, unrealized PnL, and holdings.
  Filter holdings by portfolio or search by symbol and name. Missing valuations
  and negative non-liability balances are identified explicitly.
- **Portfolios** shows account totals and unrealized PnL, and lets you create
  empty portfolios. Open a portfolio for its net worth, assets, liabilities,
  unrealized PnL, holdings, and links to filtered transactions.
- **Transactions** has portfolio, asset, and text filters, with 50 entries per
  page. Asset scope can match the primary asset or any side, including quote
  currencies and fees. Each row's **Edit** link opens that transaction. Update
  its fields or use **Delete transaction** on the editor to remove it. Notes appear
  directly in the table, with line breaks preserved. The edit page uses the same
  two-column layout as the asset editor, with entry, source, exchange, and broker
  details in a side panel. The sidebar stacks below the form on smaller screens.
- **Add transaction** selects an existing portfolio by ID, with primary, quote,
  and fee assets selected from the asset directory. Asset choices show their
  symbol, name, and type, with IDs to distinguish duplicate symbols. Create an
  asset in **Assets** and a portfolio in **Portfolios** before using them in a
  transaction. Dates and times are entered in UTC. Validation errors preserve
  entries and selections. Notes are always visible in the editor's **Transaction**
  section; fees, exchange, and broker fields are in a collapsible section.
- **Assets** lists instruments and recorded prices. Search by name, type, or
  provider symbol and refresh Yahoo prices. Each asset has
  an editor for its display name, type, Yahoo symbol, TradingView symbol, and
  intrinsic valuation currency, alongside its recorded quotes and transaction
  links. Record manual prices in the editor after saving an asset without a Yahoo
  symbol. Manage dated stock splits in the same editor. Assets can be created
  before their first transaction.
- The **Base** selector switches between configured valuation currencies on
  change and requires JavaScript.
- **Privacy** in the header hides totals, PnL, prices, transaction amounts, fees,
  and quantities behind the same six-dot mask, regardless of size or sign. It
  remembers your choice in this browser across navigation and reloads. Amount
  fields are hidden while privacy is on; turn it off to edit them. This
  screenshot mode requires JavaScript. Names, notes, and other text stay visible.
- Search filters submit automatically after a 300 ms typing pause or immediately
  when a filter dropdown changes. Use **Reset** to clear the filters.
- Holdings link to their transactions and configured TradingView charts. Manage
  asset settings through the **Assets** section.

Other forms and navigation also work with JavaScript disabled. Submit search
filters with Enter when JavaScript is disabled. Monetary amounts stay as decimals
in Rust; quantities retain their full precision. Fiat valuations display two
decimal places and crypto valuations display eight.

## Accounting and storage

The ledger-first JSON datastore lives in the platform's local data directory
under `tuifolio/store.json`, unless you supply `--store`.

Existing datastore paths resolve to their canonical target before locking and
saving. Symlink aliases share the target's lock and remain symlinks after saves.
New stores resolve their parent directory first. Dangling file symlinks are
rejected rather than replaced.

Loading and saving validate structural IDs, references, allocator bounds, split
configuration, and reporting currencies. Invalid structures are rejected without
rewriting the input. This does not apply new-transaction amount/quote rules to
all historical records: supported ignored deposit quote metadata stays intact.

On Unix, saving synchronizes the replacement file and its containing directory.
If directory synchronization fails after replacement, the error explicitly says
the store was saved but crash durability is uncertain. The web app retains that
saved state in memory rather than claiming the edit was rolled back. Other
platforms log that directory synchronization is unavailable; crash durability
there is not guaranteed.

- Tracks portfolios, assets, transactions, ledger entries, and prices.
- Every transaction posts its asset movement to ledger balances. Buys, sells,
  and quoted asset increases/decreases can optionally post their cash movement.
- Ledger balances rebuild from transactions when opening the datastore and after
  transaction edits, so stored ledger entries follow the current accounting rules.
- Property and mortgage liabilities remain separate assets.
- Configured stock splits apply when the ledger is rebuilt. They target asset IDs;
  no ticker-specific split events are inserted automatically.
- Yahoo quotes support crypto, stocks, funds, fiat exchange rates, and
  commodities through per-asset provider symbols.
- USD is the fiat conversion intermediary and must use the Cash asset type.
  Fiat-to-crypto refreshes resolve the USD step directly, without recursion.
  If an existing store misclassifies USD, refresh returns an error before
  contacting Yahoo. The web server stays available, and cached prices remain
  unchanged. Correct the USD asset type in its editor before refreshing again.
- PnL is derived from transactions, ledger entries, and prices. It is not
  persisted as canonical accounting truth.
- Every holding without a valuation counts as unresolved, including cash and
  liabilities. Web and CLI reports mark their balance-sheet totals as partial
  and explain that these holdings are excluded. The CLI `missing-prices`
  command includes them.
- Valuation completeness is separate from investment PnL availability. If
  investment holdings exist but none has computable PnL, the aggregate is
  unavailable rather than zero. A computable subtotal is marked partial when
  other investment holdings lack a valuation or recorded cost basis. Cash and
  liabilities do not require acquisition basis for this investment metric.
  These are reporting checks, not stored fields or a datastore migration.
  Complete valuation does not certify quote freshness or correct cost-basis
  accounting.

Manual buys and sells require an asset quantity, an existing quote asset, and a
total quote amount. The quote amount supplies cost basis or proceeds for PnL.
Quote amounts may be zero but cannot be negative; asset quantities must remain
positive. An explicit zero quote records a known zero cost basis and needs no
exchange rate for that cost basis.
If no cost basis has been recorded for a holding, its PnL is unavailable. Zero
quotes create no cash movement, even with **Post cash movement** selected.

**Gift** records an incoming asset with an implicit zero acquisition cost. Enter
the positive quantity received; no quote currency, amount, or cash effect is
needed. Its zero cost basis is derived from the transaction type, so the stored
quote fields remain empty. The transaction list shows `0` and **Zero cost basis**.
Gifts contribute zero to net invested and create no quote-side cash movement.
Fees can still be recorded and deduct from the selected fee asset.

**Asset increase** accepts an optional quote amount as acquisition cost, like
**Buy**. **Asset decrease** accepts an optional quote amount as proceeds, like
**Sell**. Costs add to net invested; proceeds subtract from it. Valuation-only
increases or decreases can omit the quote without changing recorded costs or
proceeds. For example, a property initially valued at 255000 EUR with a 420000 EUR
cost, followed by an unquoted valuation increase of 245000 EUR, has a 500000 EUR
value, 420000 EUR net invested, and 80000 EUR PnL.

The quote and cash fields appear only for **Buy**, **Sell**, **Asset increase**,
and **Asset decrease**. Changing the type hides and disables inapplicable fields;
their values are omitted on save. **Deposit**, **Withdraw**, **Liability increase**,
**Liability decrease**, and **Gift** reject quote assets and amounts. Old quotes
on deposit, withdrawal, and liability types are ignored when calculating PnL and
do not establish a zero cost basis.

Transactions reference portfolio and asset IDs and never create portfolios or
change asset definitions.
Fee assets and fee amounts must be supplied together.

Editing keeps the transaction ID and historical origin fields, then rebuilds
the ledger. Creating and editing use the same validation rules. Existing
records with invalid amounts or incomplete buy/sell quotes need correction
before an edit can be saved; they can still be viewed and deleted. Loading a
store does not repair or reject these records automatically.

The editor preserves the cash effect setting and timestamp
precision on unchanged dates. Deleting removes the transaction and its ledger
effects, while retaining asset definitions, portfolios, and prices. Failed saves
roll back the edit or deletion.

The transaction form's cash effect has two choices:

- **Cost basis only** records the quote amount for PnL without changing cash
  holdings. Use this when broker or exchange cash balances are not tracked.
- **Post cash movement** also posts the quote side. Buys and asset increases
  reduce cash; sells and asset decreases increase it. Use this when cash deposits,
  withdrawals, and balances are tracked.

Fees apply to every transaction type and always post to the ledger when supplied.

### Asset settings

Edits keep an asset's ID, so its transactions, ledger quantities, historical
prices, and stock splits keep their references. Local symbols are fixed after
creation; display names and provider mappings remain editable. Provider fields
are used exactly as configured. Blank mappings stay blank across restarts and
transactions, regardless of an asset's historical metadata-source flag.

Yahoo and TradingView symbols are entered explicitly. The editor checks their
format, but does not search or verify instruments against those providers.
Crypto Yahoo symbols must quote in USD, such as `BTC-USD`. TradingView symbols
include their exchange, such as `NASDAQ:AAPL` or `CRYPTO:BTCUSD`. Clear a Yahoo
symbol to use manual market prices; clear a TradingView symbol to remove the
chart link. Cash uses exchange rates independently of its Yahoo symbol field.

Yahoo pence units `GBp` and `GBX` normalize to GBP by dividing by 100.
Provider numeric values are parsed directly as decimals, including scientific
notation, without an intermediate binary float. Unsupported provider units are
not stored as usable observations.

Manual price recording lives on each asset's edit page and uses that asset's ID.
An asset with a Yahoo symbol cannot accept new manual prices, including through
the CLI. Clear the Yahoo symbol and save first. Existing quotes are retained when
provider settings change. Cash exchange-rate refreshes can still replace manual
cash quotes. Zero market prices are allowed for non-Cash assets, while Cash
exchange rates must remain positive. Zero-valued crypto observations cannot be
used as conversion denominators.

For property, liabilities, and custom assets, an intrinsic valuation currency
means each unit of quantity equals one unit of that currency. It takes
precedence over market prices. Leave the field blank to use recorded prices.
The input appears only for these types and updates when the type changes with
JavaScript enabled. Switching to another type omits intrinsic valuation on save.

Currency assets used in accounting must retain a Cash or Crypto type. Their
display names and provider settings remain editable. CLI and web reporting
currency changes share validation and normalize identifiers to uppercase.
Reporting accepts recognized fiat codes or unique Cash/Crypto identities, not
arbitrary stock symbols. Configured BTC and ETH units remain available even
before any holdings exist.

### Stock splits

Open an existing asset's editor and use **Stock splits** to add an effective date
and a ratio of new shares to old shares. For example, `4 : 1` multiplies quantities
by four, while `1 : 10` records a reverse split. Expand a recorded split to edit
its date or ratio, or use **Delete split** to remove it. Splits are listed newest
first and are scoped to the asset's ID, even when assets share a symbol.

Saving or deleting a split rebuilds ledger balances immediately across portfolios.
It adjusts every posted ledger leg in that asset, including quote and fee legs,
for transactions before the effective date in UTC. Transactions on that date are already treated as post-split. The date
is a transaction cutoff, not a scheduled job; future-dated splits also apply
immediately. Original transactions, quote amounts, fees, and recorded prices are
retained. Multiple splits compound under the existing ledger rules. Derived
market prices are adjusted inversely when their recorded timestamp predates a
split, keeping their units consistent with the rebuilt quantities. Post-split
observations are not adjusted again.

Dates must use `YYYY-MM-DD`; ratio values must be positive decimals and must change
the quantity. Only one split per asset and date can be saved. Failed validation,
ledger rebuilds, or disk writes retain the current data. Edits and deletions check
the original split values and reject forms for splits that have since changed.

### Portfolios

Create portfolios from **Portfolios → Create portfolio** or the link in the
transaction form. Names are trimmed, limited to 200 characters, and checked for
duplicates without regard to ASCII letter case. Transactions select existing
portfolio IDs, including distinct legacy portfolios that share a name. Portfolio
creation does not create assets or transactions.

Portfolio PnL sums the holdings with both a valuation and a recorded cost basis,
using the selected base currency. The portfolio list and detail totals show the
same value, with positive and negative amounts colored like the overview.

### Saving data

Saves atomically replace the JSON file. Web edits become active after that
replacement. Failures before replacement retain the previous data. A directory
synchronization failure after replacement reports that the edit was saved but
its crash durability is uncertain; active state still matches the saved file. Price refreshes fetch outside the datastore lock, then merge
their quotes into the current data, preserving edits made during the refresh.
Quotes fetched with provider settings that changed during a refresh are discarded.

Transaction and asset editors include the record version loaded by the form.
Saving checks that version while holding the datastore lock. If the record has
changed in another tab, or the submitted version is missing, the server returns
HTTP 409 and preserves the submitted form without saving it. Copy your draft,
open the latest saved record, and reapply your changes. Retrying the stale form
does not bypass the conflict check. Unrelated edits and price refreshes do not
invalidate a record's version. This works with and without JavaScript and adds
no datastore fields.

One Tuifolio process owns a datastore at a time. Stop the web server before
running CLI commands against the same datastore. The adjacent `.lock` file is
normal; the operating system releases its lock when the process exits.

### Legacy datastore fields retained for later cleanup

CSV processing and its runtime workarounds are gone. The historical fields below
remain in the datastore.

| Field | Current behavior | Later cleanup |
| --- | --- | --- |
| `raw_rows` | Historical row payloads load and save unchanged. Nothing parses, hashes, or consults them for deduplication. | Remove the field and its stored payloads through an explicit schema/data migration. |
| Transaction `source` | Historical origin labels are preserved and can appear in transaction details. New transactions use `manual`. Origin never affects validation, balances, or valuation. | Decide whether to keep general provenance or remove old import labels. |
| Transaction `source_row_hash` | Existing values survive edits. New transactions still write `manual:<id>` to satisfy the retained field. It is not used for matching or deduplication. | Remove the field or replace it with a general-purpose identifier if needed. |
| Asset `metadata_source` | The `Automatic`/`User` values remain readable and writable; new or edited assets use `User`. The flag no longer controls provider inference or metadata reconciliation. | Remove the flag and its enum when the storage schema is cleaned up. |

Existing transactions originally created from CSV, including property/mortgage
history, remain ordinary ledger records. This change does not rewrite or delete
them, clear raw rows, or reset IDs.

Quote/cash posting effects, intrinsic valuation currencies, prices and their
sources, and asset-ID stock split events are active accounting features, not
obsolete import fields. Existing configured split events continue to affect balances.

There is no automatic datastore migration. Transactions and stock splits must
use the current format; unknown fields are rejected rather than silently dropped.

The asset-side `base_ledger_effect` field and "Record only" option are unsupported.
Stores containing that transaction field fail to load, including when its value
is `Post`. Asset movements always post in the supported format.

Every stock split must explicitly reference an `asset_id`. Symbol-based splits
are rejected, even if an asset ID is also present. Current asset-ID splits
continue to work unchanged.

If loading fails on these old fields, use a compatible store snapshot or
reconcile and convert a separate copy before using it. Do not simply delete an
`Ignore` posting flag or a split symbol to bypass the error; that can change
balances. A rejected store is not rewritten.

## CLI commands

```sh
cargo run -- sync-prices
cargo run -- summary
cargo run -- holding BTC
cargo run -- missing-prices
cargo run -- add-price GOLD 3000 EUR
cargo run -- base BTC
```

`base` selects a base currency and adds it to the configured choices if necessary.

## Development

Web handlers and view models live in `src/web/`, HTML templates in `templates/`,
and embedded browser assets in `static/`. Rebuild Rust after changing templates
or assets. Vendored HTMX 2.0.8 and IBM Plex Sans have adjacent license files.
The stylesheet uses a 17 px body font and 48 px standard controls, with larger
spacing throughout. Responsive breakpoints are 1440, 1080, and 720 px.
Tests construct assets, portfolios, transactions, and split events directly;
they do not depend on CSV files or a personal datastore.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
