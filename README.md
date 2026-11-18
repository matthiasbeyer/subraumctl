# subraumctl

Rust CLI and keyboard-driven TUI for the subraum MQTT control system.

> [!CAUTION]
> This is completely slopped, because we do not care about it.
> The current system is so utterly bad, that even this slop-bomb is lightyears
> better.
> I spend about $1 USD on it with deepseek.

[MCP server usage and permissions](docs/mcp.md)

[Heating controls and per-room status in the TUI and MCP](docs/heizung.md)

[Room climate readings in the CLI, TUI and MCP](docs/climate.md)

[Power consumption readings in the CLI, TUI and MCP](docs/power.md)

[Pizzamat ordering in the TUI and MCP](docs/pizzamat.md)

[MPD music playback and radio stations in the TUI, CLI and MCP](docs/music.md)

[Searchable Soundboard tab over EDI AMQP](docs/soundboard.md)

[Strichliste 0.11.0 accounts, payments and transfers in the CLI, TUI and MCP](docs/strichliste.md)

# Issues

Issues are disabled. If you find an issue, send patches.

# Build features

TUI mouse interaction is optional and disabled by default. Build with
`--features tui-mouse` to include it.

Strichliste CLI and TUI are included by default. Build with `--features mcp`
for its restricted MCP tools. Use `--no-default-features` to omit Strichliste.

MCP server support is optional and disabled by default. Build with
`--features mcp`, or `--features mcp,tui-mouse` to enable both features.

# Release builds

Every push to `master` (including merged pull requests) builds and tests native
Linux binaries for x86_64 and aarch64, then publishes four `.tar.gz` archives and a
`SHA256SUMS` file as a GitHub prerelease in this repository's **Releases** page.
Each architecture has two variants, both with MCP and Strichliste support:

| Archive suffix | Enabled features |
| --- | --- |
| `-mcp.tar.gz` | `strichliste,mcp` (no TUI mouse support) |
| `-mcp-mouse.tar.gz` | `strichliste,mcp,tui-mouse` |

All four builds must pass before publishing.

Versions use the Cargo package version followed by `-g` and the first 12
characters of the commit hash, for example `0.1.0-g7291030abcde`, with a
`v0.1.0-g7291030abcde` release tag. This is a SemVer prerelease, and the binary's
`--version` reports the same version. Version stamping happens only in the
release build checkout; the committed Cargo version remains the base version.

Archives contain `subraumctl` and this README, preserving the binary's executable
permissions. The binaries require Linux with glibc 2.39 or newer and OpenSSL 3,
such as Ubuntu 24.04. Download the archive for your architecture and preferred
mouse variant, along with `SHA256SUMS`, verify it with
`sha256sum --check --ignore-missing SHA256SUMS`, and extract it with `tar -xzf`.
Enter the extracted directory and run `./subraumctl --help`.

# License

WTFPL
