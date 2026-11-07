# Power consumption

Read meter power for **L1**, **L2**, **L3** and **Total** in watts. The integration
uses the shared InfluxDB reader from climate, independently of MQTT and heating.
Stored values are divided by **1000**, following the supplied original display;
fractional watts are preserved in JSON and shown to two decimals in the UI.
Negative values are preserved. Total comes from the meter's `P∑` field, rather
than being reconstructed from the phases.

## TUI

```sh
subraumctl tui --view power
subraumctl --dry-run tui --view power --no-connect
```

Open **Power** from the sidebar or Overview. **←/→** or **m** switches between
15-minute averages and latest readings; **Enter** refreshes. The view reads on
entry and polls every 30 seconds while active, with at most one power request
in flight. A mode change during a request discards the old-mode response and
then reads the requested mode.

The display shows refresh age and, in latest mode, meter sample age; samples
older than 15 minutes are marked `>15m`. Failed refreshes keep the previous
snapshot with a **STALE** error label and its actual sampling mode. Missing
readings are `? W`, not zero.

`--no-connect` disables automatic reads; explicit refreshes and mode changes
still work. `--dry-run` prevents HTTP requests. MQTT `c/d/r` shortcuts retain
their normal meaning. Reads and decoded responses appear in the traffic pane.

### History graphs (TUI only)

```sh
subraumctl tui --view power --graph-layout overlay --graph-minutes 360
subraumctl tui --view power --graph-layout separate --graph-minutes 1440
```

**g** cycles table → colored overlay of L1/L2/L3/Total → separate charts → table.
**+ / -** adjusts the history window, **↑/↓** scrolls separate charts, and **Enter**
refreshes. Graphs have named legends and auto-scaled watt axes, including negative
values; Total is read from `P∑`, never calculated from the phases.

`--graph-minutes` sets any whole-minute window from 1 to 43200 (default: 60).
`--graph-layout` defaults to `table`; `SUBRAUMCTL_GRAPH_MINUTES` and
`SUBRAUMCTL_GRAPH_LAYOUT` set persistent startup defaults. History is read from
InfluxDB using roughly 600 mean buckets per series, with gaps for missing data.
The table's average/latest setting does not affect graphs. Active graphs refresh
every 30 seconds; failures retain the previous graph with its original range and
a **STALE** label. The x-axis shows minutes relative to the query end, with its age.

The same [graph controls and offline/dry-run behavior](climate.md#history-graphs-tui-only)
apply to climate. Each section remembers its window and layout for the session.

## CLI

```sh
subraumctl power status
subraumctl power status --mode latest
subraumctl --json power status --mode average
subraumctl power status --watch --interval 30
subraumctl --dry-run power status
```

`get` aliases `status`. The default mode is `average`. Watch waits the configured
interval after each completed read and stops on Ctrl+C or a read error.
`--quiet` suppresses human output. `--json` emits one object per snapshot:

```json
{
  "mode": "latest",
  "queried_at_ms": 1794000001000,
  "sample_time_ms": 1794000000000,
  "window_minutes": null,
  "l1_watts": 1234.567,
  "l2_watts": 1000.0,
  "l3_watts": 2000.0,
  "total_watts": 4234.567
}
```

Missing readings are `null`. `queried_at_ms` and `sample_time_ms` are Unix
milliseconds. Averages have `window_minutes: 15` and `sample_time_ms: null`;
InfluxDB's aggregate timestamp is not treated as a meter sample timestamp.
No series/rows yields null readings and no sample timestamp. Dry runs return
the database, mode and query without connecting.

## MCP

`power_status` accepts `{}`, `{"mode":"average"}` or `{"mode":"latest"}` and
returns the CLI's snapshot shape. It is one-shot only. Permission group `power`
defaults to `r`; `x` hides/disables the tool. `w` also permits reads, but there
are no power write operations. The endpoint, query, database and timeout are
configured at server startup, not through tool arguments.

## InfluxDB protocol

Uses the same `--influx-url` / `SUBRAUMCTL_INFLUX_URL` and `--influx-timeout` /
`SUBRAUMCTL_INFLUX_TIMEOUT` options as [climate](climate.md), defaulting to
`http://fritteuse.subraum.c3pb.de:8086/query` and 3 seconds. No authentication is
required.

- Database: `subraum-stromzähler`
- Measurement: `modbus`
- Filter: `"name" = 'stromzähler'`
- Fields: `P1`, `P2`, `P3`, `P∑`
- **average:** mean of each field over the same last-15-minute window.
- **latest:** the newest meter record at or before the query time, selected
  with `ORDER BY time DESC LIMIT 1`. All fields come from that record, sharing
  its timestamp; a missing phase stays null. There is no lower time bound, so
  older meter records remain visible with their age.

Queries alias fields to stable column names and use `epoch=ms`. Unicode
database names, tags and fields are URL-encoded by `src/influx.rs`. HTTP errors,
invalid JSON, query errors, malformed rows and partial results fail the read.

## Verification

`cargo test` and `cargo test --features mcp` cover local HTTP fixtures, Unicode
query encoding, scaling, missing values, timestamps, CLI output, TUI polling and
mode changes, and MCP permissions/validation. Live meter access is not needed.
