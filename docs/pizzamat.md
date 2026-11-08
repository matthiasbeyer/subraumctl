# Pizzamat

Pizzamat is available in the default TUI build and in MCP-enabled builds. It
talks directly to EDI over AMQP. Orders remain on the Pizzamat service.

## Setup

```sh
export SUBRAUMCTL_PIZZAMAT_USER='joni'
subraumctl tui --view pizzamat
# Or, with an MCP-enabled build:
subraumctl mcp
```

Open **Pizzamat** in the sidebar. The existing `--view` option can also open it
directly: `subraumctl tui --view pizzamat`; no feature flag is needed.
The default endpoint is `amqp://amqp.subraum.c3pb.de:5672/%2f` (vhost `/`).
Override it with `SUBRAUMCTL_PIZZAMAT_AMQP_URL` or `--pizzamat-amqp-url`:

```sh
export SUBRAUMCTL_PIZZAMAT_AMQP_URL='amqp://your-edi-broker:5672/%2f'
```

No username/password configuration is needed for the default deployment. The
URL omits credentials; the AMQP library uses its default guest authentication.
MQTT's `--broker`, `--username` and `--password` apply only to MQTT.

`--pizzamat-user` is the equivalent identity flag. Identity is the exact key
Pizzamat uses for personal orders and defaults, not a broker login. Set it to
the same identity used by your chat bot to manage those orders. MCP uses the
startup identity for every call; tool arguments cannot select another user.
The TUI's first action edits identity.

Commands use the production `pizza-*` names regardless of how subraumctl was
built. Use `--pizzamat-debug` for a debug Pizzamat server using `puzza-*` names.
`--pizzamat-timeout` defaults to 15 seconds per connection/reply phase (1–120),
allowing time for the order-start announcements. No requests are retried
automatically: a timeout can happen after an order change, so check the list
before repeating a write. The EDI `cmd` and `msg` exchanges must already exist.

To preview commands without sending them:

```sh
subraumctl --dry-run --pizzamat-user alice tui --view pizzamat --no-connect
```

The existing `--no-connect` flag starts MQTT disconnected. Pizzamat opens its
own AMQP connection per request and also works with MQTT disconnected.

## TUI

Select an action with Up/Down and press Enter. Enter advances through text
fields; Escape cancels. All characters, including `q`/`Q`, are accepted in text
fields; Ctrl+C still quits. PgUp/PgDn scroll the response; Home returns to its
top. Pizzamat actions run immediately, including when safe mode is on. For
actions with fields, the final Enter submits the command; Escape cancels the
unfinished form. The TUI's MQTT connect/disconnect controls do not affect
Pizzamat requests.

The typical sequence is **Show delivery services → Start order → Add food to my
order → Finish my order**. **End ordering** marks the shared list as ordered.
It does not submit an order to a restaurant. **Reopen ordering** clears that
marker. **Reset everyone's orders** clears the session, including its service.

**Remove matching food** removes all personal items containing the given
case-sensitive substring. **Remove my entire order** only removes your entry.
Adding/removing food marks your order uncommitted; finish it again afterwards.
**Join** also marks an existing order uncommitted. Set your usual order per
service, then add `default` to use it. Service management supports quoted names,
descriptions and URLs; use `-` for optional empty fields.

Replies are displayed verbatim, including backend rejections such as “already
ordered”. After a write, the TUI also fetches the current orders. Reply receipt
does not itself mean the requested change was accepted; read the response.

## MCP

The `pizzamat` permission group defaults to `w`; `r` exposes only list/services,
and `x` hides and rejects all Pizzamat tools. The read tools expose participant
names and food choices, as returned by Pizzamat.

| Tool | Arguments |
|---|---|
| `pizzamat_list`, `pizzamat_services` | none |
| `pizzamat_start` | `{"service":"pizza-place"}` |
| `pizzamat_join`, `pizzamat_commit`, `pizzamat_clear` | none |
| `pizzamat_add` | `{"food":"Margherita"}` or `{"food":"default"}` |
| `pizzamat_remove` | `{"food":"Margherita"}` (substring) |
| `pizzamat_lock`, `pizzamat_unlock`, `pizzamat_reset` | none |
| `pizzamat_set_default` | `{"service":"pizza-place","food":"Margherita"}` |
| `pizzamat_service_add` | `{"name":"pizza-place","realname":"Pizza Place","description":"...","order_url":"https://...","menu_url":"https://..."}` |
| `pizzamat_service_remove` | `{"service":"pizza-place"}` |

Only `name` is required for service-add; omitted optional fields are cleared.
Responses contain `command`, `submitted`, `dry_run` and the backend's `message`.
`submitted: true` means a reply was received, not that the backend accepted the
change. `--dry-run` returns the validated command/identity/arguments without
connecting. No broker address or credentials are included in tool results.

## Checking a timeout locally

Open the TUI and select **Show orders** for a read-only round trip:

```sh
subraumctl --pizzamat-user joni tui --view pizzamat --no-connect
```

- **Connection failed/timed out:** check DNS, access to port 5672, the AMQP
  endpoint/vhost, and broker logs.
- **Reply binding failed:** check that EDI's `msg` exchange exists in that vhost.
- **Command not routed:** check that Pizzamat is running and bound to `cmd` for
  `pizza-*`. Use `--pizzamat-debug` only for a server using `puzza-*`.
- **Reply timed out:** check service logs for the command and its reply. A broker
  acknowledgement confirms routing, not that Pizzamat completed the operation.

## Protocol and development

Each request creates an exclusive, auto-delete AMQP reply queue bound to `msg`
with a unique routing key before publishing to `cmd`. The command routing key
is `pizza-<operation>` (or `puzza-<operation>` in debug mode). The JSON envelope
contains `cmd`, `args`, `user`, `src`, and `dst: null`. EDI routes replies to
`src` with `recv` replaced by `send`; replies contain `msg`, `user` (the reply
destination), and `data`. Only the message is returned to the caller. Publisher
confirms with mandatory routing detect unrouted commands. Connection closure
removes the reply queue, including on timeout/error.

The local `pizzamat/` and `edi-rust/` folders are ignored protocol references,
not build dependencies or bundled services. Run the normal test suites with
`cargo test` and `cargo test --features mcp`. The AMQP integration test can be
run against a disposable broker as described in `tests/pizzamat_amqp.rs`.
