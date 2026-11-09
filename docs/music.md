# Music (MPD)

Control the MPD server on the music host. The default endpoint is
`audiomaster:6600`; playback status, the current title and radio stations are
read and changed directly over the MPD TCP protocol, independently of MQTT.

## TUI

```sh
subraumctl tui --view music
subraumctl --dry-run tui --view music --no-connect
```

Open **Music** from the sidebar or Overview. The upper panel shows the playback
state, current artist/title (or stream name), elapsed/total time and volume.
The lower list contains the playback controls and the radio station catalogue.

- **↑/↓** selects a control or station; **PgUp/PgDn** jumps five entries;
  **Home/End** goes to the first/last entry.
- **Enter** activates the selected control or starts the selected station.
- **Space** toggles play/pause using the last status. **n** and **p** send
  next and previous without moving the selection.
- **Refresh** reads status without sending a playback command.

Status is read when the section is opened and every five seconds while it is
active. `--no-connect` disables those automatic reads; explicit controls and
**Refresh** still work. A failed read keeps the last status visible with the
error message. The MQTT `c/d/r` shortcuts keep their normal meaning.

## CLI

```sh
subraumctl music status
subraumctl music status --watch --interval 5
subraumctl music play
subraumctl music pause
subraumctl music next
subraumctl music previous
subraumctl music stations
subraumctl music station groovesalad
subraumctl --json music status
subraumctl --dry-run music station hardbass
```

`status` prints the state, the current title and time/volume when available.
`stations` lists station ids and names. `station` replaces the MPD queue with
the station stream and starts playback. `prev` aliases `previous`, and
`play-station` aliases `station`. Watch waits the configured interval after
each completed read and stops on Ctrl+C or an error. `--quiet` suppresses human
output; `--json` emits one object per operation. Missing metadata is `null`.

The JSON status shape is:

```json
{
  "state": "play",
  "state_label": "Playing",
  "display_title": "Artist - Title",
  "artist": "Artist",
  "title": "Title",
  "name": null,
  "file": "http://example/stream",
  "elapsed_seconds": 12.5,
  "duration_seconds": 125.0,
  "time": "0:13 / 2:05",
  "volume": 50
}
```

## Stations

The catalogue combines a snapshot of SomaFM's `channels.xml` with the stations
from the original subraum music actor. Ids are stable and case-sensitive.
Starting a station sends:

```text
clear
add "<resolved stream URI>"
play
```

Remote `.pls`, `.m3u` and `.m3u8` URLs are downloaded by subraumctl, parsed,
and replaced with the first direct HTTP(S) stream URL before MPD sees them.
This avoids relying on MPD's remote playlist support. Direct stream URLs are
passed through unchanged. Playlist downloads use the `--mpd-timeout` value
(minimum 5 seconds).

The two `hardbass` entries refer to files in the MPD library on the music host;
the paths must exist there. The original `tts` announcements are not reproduced
by subraumctl.

## Connection and protocol

| Option | Environment | Default |
|---|---|---|
| `--mpd-address` | `SUBRAUMCTL_MPD_ADDRESS` | `audiomaster:6600` |
| `--mpd-password` | `SUBRAUMCTL_MPD_PASSWORD` | none |
| `--mpd-timeout` | `SUBRAUMCTL_MPD_TIMEOUT` | `3` seconds (1–30) |

Each operation opens one TCP connection, reads the `OK MPD` greeting, sends
`password "..."` when configured, then executes one or more commands. Replies
end with `OK`; `ACK` errors fail the operation. Commands and arguments are
single-line; station URIs and passwords are quoted for MPD. The password is
never printed or included in errors.

`--dry-run` validates the operation and prints the command without connecting.
It does not read status.

## MCP

The `music` permission group defaults to `w`; set `music = "r"` for status and
the station catalogue only, or `x` to hide and disable every music tool.

- `music_status`: read playback state, title and time.
- `music_stations`: static station ids and names.
- `music_play`, `music_pause`, `music_next`, `music_previous`: no arguments.
- `music_station`: `{"station":"groovesalad"}`.

The MPD address, password and timeout are configured at server startup, never
through tool arguments. Unknown stations and extra fields are rejected. Write
results mean the command was accepted by MPD, not that audio is audible.

## Verification

`cargo test` and `cargo test --features mcp` cover station-id validation,
status parsing, MPD command quoting, a fake MPD TCP server, dry runs, TUI
controls/rendering and MCP permissions. A live `audiomaster` connection is not
needed.
