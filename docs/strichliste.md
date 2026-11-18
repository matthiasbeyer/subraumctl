# Strichliste 0.11.0

CLI and TUI integration for the deployed legacy 0.11.0 variant, including its
transfer/comment extension and `weightedCountOfPurchases` user field.

## Build and configure

```sh
cargo build --release
# Or include restricted MCP tools:
cargo build --release --features mcp
```

Nix packages: `.#subraumctl` and `.#subraumctl-mcp`.
The `strichliste` feature is enabled by default and includes both CLI commands
and the TUI tab. Build with `--no-default-features` to omit Strichliste commands,
the tab, and its connection options.

Set your numeric account id. The API URL defaults to
`https://mate.wahrhe.it/strich-api`:

```sh
export SUBRAUMCTL_STRICHLISTE_USER_ID=27
# Optional URL override:
# export SUBRAUMCTL_STRICHLISTE_URL='https://your-server/strich-api'
```

Equivalent global flags are `--strichliste-url` and `--strichliste-user-id`.
Account selection is by numeric id only. The former username flag and
`SUBRAUMCTL_STRICHLISTE_USER` environment variable are no longer supported.
HTTP requests time out after five seconds; override with `--strichliste-timeout`
or `SUBRAUMCTL_STRICHLISTE_TIMEOUT` (1–120 seconds).

## CLI

```sh
subraumctl strichliste balance
subraumctl strichliste rank
subraumctl strichliste transactions
subraumctl strichliste pay 1.25 --comment 'Mate'
subraumctl strichliste add 10 --comment 'cash deposit'
subraumctl strichliste send 0.01 --comment 'test'
subraumctl strichliste pay 3.141592653589793
```

Amounts are positive numeric **euros**. `pay` debits, `add` credits, and `send`
debits your account and credits the selected recipient in a single API request.
The API also uses euros: `pay 0.01` sends `"value": -0.01`, and `add 1` sends
`"value": 1`. JSON write results and previews report the signed `value_eur`.
Fractional cents are preserved as floating-point values, not rounded to two
decimal places; `pi` is not a literal expression understood by the CLI.
Zero, negative, non-finite and overflowing amounts are rejected before IO.

`send` requires a terminal: enter part of a recipient's name, then choose their
number from the results. Your own account is excluded. Enter `0` at the number
prompt to search again, or empty input/EOF at either prompt to cancel without a
transaction. More than 20 matches require a narrower search.

Rank is `1 + number of users with a strictly higher weightedCountOfPurchases`.
Ties share a rank (1, 2, 2, 4); balances do not affect rank. Explicit null scores
mean no purchases and count as zero. Missing or malformed scores fail the
operation instead of producing an incomplete rank.

`--json` emits a JSON result; `--quiet` suppresses human results. Interactive
prompts go to stderr. `--dry-run` performs no HTTP requests, including no user
lookup: it previews values/comments, with recipient selection left unresolved.

`transactions` loads **50 entries by default** from
`/user/<id>/transaction?limit=50&offset=0`, newest transaction id first.
It includes date, signed euro amount,
pay/add or transfer label, and comment. Human output rounds amounts to cents;
`--json strichliste transactions` preserves the full `value_eur` precision.
Use `transactions --limit 100 --offset 100` for another page. Limits range from
1 to 1000. JSON also includes `offset`, `overall_count` (when supplied by the
server), and `next_offset` (null at the end).

Legacy transfer labels are inferred from the `to <name>: …` / `from <name>: …`
comment convention and the amount's sign. The original comment is shown so that
the recipient/sender and booking note remain visible.

## TUI

With the same URL and account environment variables configured:

```sh
cargo build
./target/debug/subraumctl tui --view strichliste
```

Choose **Strichliste** in the sidebar or start directly with `--view strichliste`.
The tab shows your balance in euros and your rank by weighted purchase count.
It refreshes automatically every 10 seconds while the tab is visible and idle;
`--no-connect` disables automatic reads. **r** refreshes immediately, independently
of MQTT, when the content panel is focused. Editing forms, pending confirmations,
and in-flight requests defer automatic refreshes.

Balances default to two decimal places (for example `-2.42 EUR`), labelled
**rounded to cents**. **p** toggles **full precision** without changing the stored
balance or any transaction amounts. JSON/MCP results retain full precision.

- Use **Up/Down**, then **Enter** to choose **Pay**, **Add credit**, or **Send**.
- Choose **Transactions**, or press **t**, to view recent account transactions.
  The first page requests **50 transactions**. **n** (or the **Load older
  transactions** row) opens the next page; **b** returns to the previous page.
  The title shows the displayed range and total. Comments are visible in each
  row and shortened with `...` to fit; narrow terminals show compact dates and
  transaction labels.
  Use **Up/Down**, **PageUp/PageDown**, or **Home/End** to browse; **Enter** opens
  the date, full-precision amount and complete comment (scroll with **Up/Down**).
  **Enter/Esc** closes the detail dialog; **t/Esc/q** returns from history to
  actions. **p** also toggles the history list's amount precision.
  History shares the 10-second account refresh and **r** refresh, keeping the
  current page. A successful write returns history to the newest page.
  Failed reads retain the last history labelled `(previous)`.
- Pay/add prompt for a positive euro amount and an optional comment.
- Send loads recipients once when opened, then filters names live as you type
  (case-insensitive). You can type while the list loads. Select a match with
  **Up/Down**, **PageUp/PageDown**, **Home/End**, then **Enter**. Your own account is
  excluded. **Backspace** edits the search; **/** clears it. Then enter the amount
  and comment. Failed loads can be retried with **Enter**.
- **Esc** cancels a form without submitting. Text fields accept global shortcut
  characters as literal text; **Ctrl+C** still quits.
- With the application's default safe mode enabled, a confirmation previews the
  amount, comment and recipient before submitting. Cancel is selected by default.
  Disabling safe mode makes completion of the comment field submit immediately.

Requests run in the background; repeated activation while busy does not enqueue
duplicate payments. Successful writes refresh balance and rank. Failed reads
keep previous values labelled `(previous)`. Unconfirmed writes are never retried.
Transaction HTTP errors include the endpoint and status code.
Failed transactions open a persistent error dialog; **Enter** or **Esc** closes it.
`--dry-run` previews operations without HTTP or invented balances/search results.

TUI payments use your comment and the existing safe-mode setting; the MCP write
opt-in and forced MCP comment apply only to MCP.

## Restricted MCP

Build with `--features mcp` and supply the same account configuration.
The `strichliste` permission group defaults to `r`:

| Tool | Arguments | Result |
|---|---|---|
| `strichliste_balance` | `{}` | `balance_eur` |
| `strichliste_rank` | `{}` | `rank`, `weighted_count_of_purchases` |
| `strichliste_pay` | `{"amount":1.25}` | `submitted`, signed `value_eur` |
| `strichliste_add` | `{"amount":10}` | `submitted`, signed `value_eur` |

Pay and add are **disabled by default**. Enable them with both:

```toml
[tools]
strichliste = "w"
```

```sh
subraumctl --strichliste-mcp-writes mcp --config permissions.toml
# Environment alternative: SUBRAUMCTL_STRICHLISTE_MCP_WRITES=true
```

Both writes always set `comment` to exactly `triggered by mcp`. Tool arguments
cannot change the account, endpoint or comment, or add recipient fields.
Transfers, user search/listing and transaction history are never registered.
Disabled tools are hidden from discovery and rejected on direct invocation.
`strichliste = "x"` also disables balance and rank.

The backend may read the user list to resolve your name or calculate rank; only
your balance/rank fields reach MCP. Raw user and transaction responses are not
returned, and HTTP errors omit response bodies and endpoint details.
`--dry-run` also supports MCP and preserves the same permissions.

## API contract and verification

The implementation uses the legacy `/user` routes, not the newer PHP API:

- `GET <base>/user`: an object with `overallCount`, `limit`, `offset`, and an
  `entries` array of user objects (`id`, `name`, `balance`,
  `weightedCountOfPurchases`). `limit` and `offset` may be null for an unpaginated
  response. Bare arrays from older variants are also accepted.
- Paginated lists are fetched with `limit`/`offset` until `overallCount` entries
  have been read. Inconsistent counts, offsets, duplicate ids, or stalled pages
  fail instead of giving incomplete ranks or recipient lists. The
  configured HTTP timeout also bounds the complete list fetch.
- `GET <base>/user/<id>`: one user object; `balance` is already in **euros** and
  is preserved without conversion or rounding.
- `GET <base>/user/<id>/transaction?limit=50&offset=0`: an `entries` array with
  `overallCount`, `limit`, and `offset` metadata (bare arrays are also accepted).
  Each entry has `id`, `userId`, `createDate`, `value`, and optional `comment`.
  Returned user ids and pagination metadata are validated before showing history.
- `POST <base>/user/<id>/transaction`: JSON with `value` in **euros** and `comment`.
  A successful 2xx response acknowledges the submission.

Numeric balances and scores may also be returned as numeric strings.
Rejected reads report the relative route and HTTP status without response bodies.
The confirmed deployed-server units are euros: `pay 0.01` posts `value: -0.01`.
A one-cent transfer from user 27 to user 30 uses:

```json
{
  "value": -0.01,
  "comment": "to Other: test",
  "toUserId": 30,
  "toUserComment": "from Me: test"
}
```

Comments use the account names returned by the API. There is no client-side
two-transaction transfer fallback. Automatic retries and redirects are disabled.
On an unconfirmed submission, check the account before retrying.

Tests use local HTTP fixtures, the supplied user-list wrapper and balance sample,
and the confirmed euro-valued transfer contract. The private deployment has not
been contacted.
