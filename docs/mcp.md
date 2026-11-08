# Restricted MCP server

## Goal and access boundary

The MCP server deliberately **cannot access everything subraumctl can access**.
Its goal is to prevent an agent from reading user data or controlling critical
equipment through unrestricted CLI/MQTT access. Only explicitly registered tools
are available, and config permissions restrict both discovery and execution.

There is no arbitrary subscription, watch/monitor, file-reading, shell, or generic
CLI tool. Device reads subscribe only to fixed, known status topics and return selected
decoded fields, never raw payloads or unknown device IDs. Broker addresses,
credentials, HTTP response bodies, and raw backend errors are not tool results.
Allowed status fields still describe the space; set their groups to `x` if that
information should not be shared.

Pizzamat reads return the ordering service's text replies, including participant
names and food choices. It uses a separate AMQP endpoint and Pizzamat order identity;
see [Pizzamat setup and tools](pizzamat.md). `pizzamat = "x"` disables this group.

Outlet control, Smoothiesaur, and raw publishing are disabled by default.
**`publish = "w"` explicitly bypasses the command restrictions:** any MQTT topic
and literal payload is permitted, even for otherwise disabled groups. It does
not read payload files. Keep it `x` to preserve the restricted command boundary.
`system_telinit` is enabled by default and can switch room systems off; set
`system = "x"` to prohibit it. These permissions govern MCP, not CLI/TUI access
or other tools granted to the agent by its host.

## Start it

```sh
cargo build --release --features mcp
./target/release/subraumctl --broker tcp://mqtt.subraum.c3pb.de:1883 \
  mcp --config ./docs/mcp.toml
```

Alternatively: `nix build .#subraumctl-mcp`, then `./result/bin/subraumctl mcp`.
MCP is absent from default builds. Even an MCP-enabled build starts the server
only with the explicit `mcp` subcommand, using stdio, not a network listener.

Configure your MCP client to launch the binary, for example:

```json
{
  "mcpServers": {
    "subraum": {
      "command": "/absolute/path/to/subraumctl",
      "args": ["mcp", "--config", "/absolute/path/to/mcp.toml"]
    }
  }
}
```

Broker/credential environment variables and global options work as in the CLI.
The Smoothiesaur URL is configured at startup, never by tool arguments. Stdout
contains only MCP messages; diagnostics go to stderr. Close stdin to stop.
Use `--dry-run mcp ...` to validate commands without any device IO: writes return
an action count and `submitted: false`; live reads return `dry_run: true`.
Static catalogues still return their data.

## Config

Copy [mcp.toml](mcp.toml) and edit `[tools]`. The file is read once at startup;
restart to change permissions. With no `--config`, or with missing entries,
the defaults below apply. No config file is auto-discovered. Unreadable files,
unknown keys, duplicate entries, and invalid permissions abort startup.

- `r`: read operations only.
- `w`: read and write operations supported by the group.
- `x`: disabled; hidden from `tools/list` and rejected on direct calls.

`r` on a write-only group disables it and prints a warning. Enabling `outlet` or
`smoothiesaur` prints that this is dangerous and might break things (including
Smoothiesaur `r`). These startup warnings also appear with `--quiet`/`RUST_LOG=off`.

| Config group | Read tools | Write tools | Default |
|---|---|---|---|
| `dmx` | — | `dmx` | `w` |
| `white_light` | `white_light_cells`, `white_light_status` | `white_light_apply` | `w` |
| `outlet` | — | `outlet` | `x` |
| `alert` | `alert_led` | `alert_radiation`, `alert_sob` | `w` |
| `system` | — | `system_telinit` | `w` |
| `knob` | `knob_list`, `knob_get` | `knob_set` | `w` |
| `smoothiesaur` | `smoothiesaur_status` | `smoothiesaur_relay`, `smoothiesaur_laser`, `smoothiesaur_zero` | `x` |
| `pizzamat` | `pizzamat_list`, `pizzamat_services` | `pizzamat_start`, `pizzamat_join`, `pizzamat_add`, `pizzamat_remove`, `pizzamat_clear`, `pizzamat_commit`, `pizzamat_lock`, `pizzamat_unlock`, `pizzamat_reset`, `pizzamat_set_default`, `pizzamat_service_add`, `pizzamat_service_remove` | `w` |
| `status` | `status` | — | `r` |
| `publish` | — | `publish` | `x` |

Examples of tool arguments: `dmx` accepts
`{"group":"subraum.all","animation":"rainbow"}`; `white_light_apply` accepts
`{"warm":128,"cold":200,"all":true}` or a `cells` array instead of `all`;
`knob_get` accepts `{"knob":"cx"}`; `knob_set` additionally takes `"value":42`.
Switches use `"state":"on"`/`"off"`. The advertised schemas list enum choices.

Live device reads are one-shot snapshots, using retained messages where available.
`--wait` bounds collection (default 3 seconds, clamped to 1–30 in MCP); missing
values are `null`, malformed numeric/boolean readings are `null`, and unknown
alert conditions become `unknown`. MQTT sessions are always clean; MCP rejects
`--no-clean-session` to prevent inherited subscriptions. Device IO is serialized.
Write success means submitted, not confirmed physical state; a failed/cancelled
multi-cell write may have submitted earlier actions. Backend errors omit raw
details so an MCP client cannot obtain them by provoking failures.
