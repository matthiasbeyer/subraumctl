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

## Reuse for the later power branch

`src/influx.rs` owns endpoint/timeout configuration, sampling mode, clock, HTTP
transport and common InfluxDB error handling. `src/climate.rs` owns climate
queries, parsing and typed snapshots. A future `power` domain can reuse the
transport and sampling modes with its own query/decoder and CLI/TUI/MCP wiring.

The supplied power reference uses database `subraum-stromzähler`, measurement
`modbus`, tag `name='stromzähler'`, and fields `P1`, `P2`, `P3`, `P∑`. Its `/1000`
conversion to watts belongs in that power decoder; verify the stored units
when adding it. Unicode database/query parameters are handled by the shared
URL encoder. No power-specific query or presentation is needed in climate.

## Verification

`cargo test` and `cargo test --features mcp` cover local HTTP fixtures, missing
and malformed data, query modes, CLI output, TUI mode changes/refresh/failures,
and MCP argument validation, dry runs and permissions. Tests do not require a
connection to the space's InfluxDB.
