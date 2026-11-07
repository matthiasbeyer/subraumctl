# Room climate

Measured temperature and humidity are read from InfluxDB, independently of MQTT
and heating. Locations come from the database's `location` tags; they do not
need to match the six heating rooms.

## TUI

```sh
subraumctl tui --view climate
subraumctl --dry-run tui --view climate --no-connect
```

Open **Climate** from the sidebar or Overview. **←/→** or **m** switches sampling
mode, **Enter** refreshes, and **↑/↓** scrolls locations. Both temperature (°C) and
humidity (%) are shown, rounded to two decimals. The view reads on entry and
refreshes every 30 seconds while active, with at most one request in flight.
Changing mode requests new data; an in-flight response for the old mode is
discarded before requesting the new mode.

`--no-connect` disables automatic reads; explicit refresh and mode changes still
work. `--dry-run` prevents HTTP requests. MQTT's `c/d/r` shortcuts retain their
usual meaning. Requests and decoded replies appear in the traffic pane.

Failed refreshes keep the last successful snapshot with a **STALE** error label.
The table always labels the mode of the displayed snapshot, even while a mode
change is loading or has failed. Missing values show `?`; no series produces an
empty-data message. Snapshot refresh age is distinct from sensor sample age.

### History graphs (TUI only)

```sh
subraumctl tui --view climate --graph-layout overlay --graph-minutes 360
subraumctl tui --view climate --graph-layout separate --graph-minutes 1440
```

- **g** cycles table → colored overlay → separate charts per room → table.
- **t** switches graphs between temperature (°C) and humidity (%), each with its
  own scale. Room colors and legend names are shared between layouts/metrics.
- **+ / -** lengthens/shortens the window using presets from 1 minute to 30 days.
- **↑/↓** scrolls separate charts when they do not all fit; **Enter** refreshes.

`--graph-minutes` accepts any whole number from 1 to 43200 (default: 60).
`--graph-layout` accepts `table` (default), `overlay`, or `separate`. Set defaults
with `SUBRAUMCTL_GRAPH_MINUTES` and `SUBRAUMCTL_GRAPH_LAYOUT`. Each section keeps
its own window and layout during the session.

Graphs query existing InfluxDB history immediately; there is no collection/warm-up
period. Queries use roughly 600 equal time buckets per series, plotting the mean
in each bucket. Missing buckets remain gaps. Graphs use bucket means independently
of the table's average/latest mode. The x-axis is minutes relative to the displayed
query's end, whose age is shown; y-axes auto-scale. Only the active graph view polls
(every 30 seconds). `--no-connect` disables automatic polling; graph selection,
range changes, and Enter still request data. `--dry-run` skips HTTP entirely.

Failed reads keep the previous graph labeled **STALE**, retaining its actual range.
While a changed window loads, the requested window and displayed range are labeled
separately. Old-window responses are discarded before loading the new window.

## Sampling modes

- **`average`** (default): mean over the last 15 minutes, matching the original
  display. Reports the query time and window, rather than treating InfluxDB's
  aggregate timestamp as a sensor timestamp.
- **`latest`**: latest non-null sample for each metric/location at or before the
  query time. There is no lower time bound: old sensors remain visible with
  their actual timestamps. Temperature and humidity may have different sample
  times. The TUI shows ages and marks samples older than 15 minutes as `old`.

## CLI

```sh
subraumctl climate status
subraumctl climate status --mode latest
subraumctl --json climate status --mode average
subraumctl climate status --watch --interval 30
subraumctl --dry-run climate status
```

`get` aliases `status`. Watch waits the interval after each completed read and
stops on Ctrl+C or a read error. `--quiet` suppresses human output. JSON produces
one snapshot per line, including `mode`, `queried_at_ms`, `window_minutes` (15
for averages, null for latest) and a sorted `rooms` array. Each entry has:

- `location`
- `temperature_celsius`, `humidity_percent` (numbers or null)
- `temperature_time_ms`, `humidity_time_ms` (Unix milliseconds for latest
  non-null samples; null for averages or missing samples)

Dry runs return the database, mode and query without connecting.

## MCP

`climate_status` accepts `{}`, `{"mode":"average"}` or `{"mode":"latest"}` and
returns the same snapshot as the CLI. It is a one-shot read. The `climate`
permission group defaults to `r`; `x` hides/disables the tool. `w` also permits
the read, but the group has no write operations. Endpoint, database, query and
timeout cannot be supplied by tool callers.

## Connection and protocol

| Option | Environment | Default |
|---|---|---|
| `--influx-url` | `SUBRAUMCTL_INFLUX_URL` | `http://fritteuse.subraum.c3pb.de:8086/query` |
| `--influx-timeout` | `SUBRAUMCTL_INFLUX_TIMEOUT` | `3` seconds (1–120) |

No authentication is required. Requests use the InfluxDB v1 HTTP GET `/query`
API with URL-encoded `db`, `q`, and `epoch=ms` parameters. The database is
`climate`; two statements select `mean("value")` or `last("value")` from `temp`
and `hum`, each grouped by `location`. Both statements use the same upper time
bound. HTTP errors, malformed JSON, query errors and malformed/partial data
fail the read rather than inventing zero values. An absent series or null
measurement is valid missing data.

## Shared sensor infrastructure

`src/influx.rs` owns endpoint/timeout configuration, sampling mode, clock, HTTP
transport and common InfluxDB error handling. `src/climate.rs` owns climate
queries, parsing and typed snapshots. The [power integration](power.md) reuses
the transport and sampling modes with its own query, decoder and CLI/TUI/MCP
presentation. Power's conversion to watts belongs in `src/power.rs`.

## Verification

`cargo test` and `cargo test --features mcp` cover local HTTP fixtures, missing
and malformed data, query modes, CLI output, TUI mode changes/refresh/failures,
and MCP argument validation, dry runs and permissions. Tests do not require a
connection to the space's InfluxDB.
