# Local usage (stdio)

[← Documentation](README.md)

For one person, on their own machine. The MCP client (Claude Code, Claude Desktop…) starts
`parfumo-mcp` and talks to it over standard input and output. Nothing listens on the
network.

## Install

**Download.** Take the archive for your system from the [latest release](https://github.com/hessbey/parfumo-mcp/releases/latest) and unpack
it. It holds `parfumo-mcp` (and `parfumo-mcp-server`, which you can ignore).

| System | Archive |
| --- | --- |
| Linux, Intel or AMD | `parfumo-mcp-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux, ARM | `parfumo-mcp-<version>-aarch64-unknown-linux-gnu.tar.gz` |
| macOS, Apple silicon | `parfumo-mcp-<version>-aarch64-apple-darwin.tar.gz` |
| macOS, Intel | `parfumo-mcp-<version>-x86_64-apple-darwin.tar.gz` |
| Windows | `parfumo-mcp-<version>-x86_64-pc-windows-msvc.zip` |

`SHA256SUMS`, next to the archives, lets you check a download. The programs are not signed:

- on macOS, a downloaded program is quarantined. Clear it once with
  `xattr -d com.apple.quarantine /path/to/parfumo-mcp`;
- on Windows, SmartScreen may warn the first time a program is run by hand. An MCP client
  that starts it itself is not affected.

The Linux programs need glibc 2.35 or newer (Ubuntu 22.04, Debian 12 and later).

**Or build it**, with Rust 1.90 or newer:

```sh
git clone https://github.com/hessbey/parfumo-mcp && cd parfumo-mcp
cargo build --release -p parfumo-mcp
# binary: target/release/parfumo-mcp (about 10 MB)
```

## Give it a Parfumo account

One account, read from the process environment. Nothing is ever typed into the conversation
with the assistant.

| Variable | Mode |
| --- | --- |
| `PARFUMO_COOKIE` | **Cookie mode**, no password: the `pf_auth=…` value copied from a browser logged in to Parfumo (the whole `Cookie` header works too). When it expires, tools return `reauth_required` and you copy a new one. Clicking "Logout" in that browser revokes the cookie |
| `PARFUMO_USER` and `PARFUMO_PASSWORD` | **Credentials mode.** The binary logs in the first time a tool needs the account, keeps the session between runs, and logs in again by itself when it expires (three attempts at most, then `reauth_required`) |

The two modes are exclusive: with both, or with half of the credentials pair, the binary
refuses to start and says why. **With neither, it still starts**: the public tools
(database, forum, blogs, Souk, profiles) work, and account tools return `not_linked`.

If Parfumo rejects the password, the binary never sends it again — even after a restart —
until the variable changes. It never solves a captcha: if Parfumo shows one, the tool
returns `captcha_required` and the way out is cookie mode.

A password written in the client's configuration is stored there in clear text. That is a
reason to prefer cookie mode.

## Connect a client

Claude Code:

```sh
claude mcp add parfumo \
  -e PARFUMO_COOKIE='pf_auth=…' \
  -- /path/to/parfumo-mcp
```

Clients configured with a JSON file (Claude Desktop and most others):

```json
{
  "mcpServers": {
    "parfumo": {
      "command": "/path/to/parfumo-mcp",
      "env": { "PARFUMO_USER": "my-nickname", "PARFUMO_PASSWORD": "my-password" }
    }
  }
}
```

Check it with MCP Inspector, no account needed:

```sh
npx @modelcontextprotocol/inspector --cli target/release/parfumo-mcp --method tools/list
```

## Settings

| Variable | Default | Purpose |
| --- | --- | --- |
| `PARFUMO_MCP_DB` | `$XDG_DATA_HOME/parfumo-mcp/parfumo-mcp.db`, else `~/.local/share/parfumo-mcp/parfumo-mcp.db` (`%LOCALAPPDATA%` on Windows) | Local SQLite file: cache of public pages, Parfumo session, audit log, daily counters |
| `PARFUMO_MCP_PUBLIC_WRITES` | `false` | `true` allows writes that other members can see to be really sent. Otherwise those tools answer as in `dry_run` |
| `PARFUMO_MCP_PUBLIC_WRITE_CAPS`, `PARFUMO_MCP_WRITE_RECIPIENTS` | as the HTTP server | Daily caps and allowed recipients: see [Configuration](CONFIGURATION.md#public-writes) |
| `PARFUMO_BASE_URL`, `PARFUMO_MCP_USER_MIN_INTERVAL_MS`, `PARFUMO_MCP_USER_JITTER_MS`, `PARFUMO_MCP_GLOBAL_RPS` | as the HTTP server | Parfumo domain and request rate (never more than one request per second) |
| `PARFUMO_MCP_MASTER_KEY`, `PARFUMO_MCP_MASTER_KEY_FILE` | — (optional) | Encryption key supplied from outside, instead of the local key file |

The HTTP server's variables about ports, public URL, OAuth tokens or the canary have no
effect here.

## Local files

- **The cache stays local**, in the file above. Nothing is shared with a server.
- The session and, in credentials mode, the password are encrypted with a key created on
  first run in the neighbouring file `parfumo-mcp.db.key`. Both files are readable only by
  their owner (mode 600). The key sits on the same disk as the database: this protection is
  worth the file permissions, no more. Deleting both files resets everything.
- To switch accounts, change or remove the variables: on the next start the stored account
  is replaced or erased.
- Standard output carries only the MCP protocol. The log (one line per tool call, no secret)
  goes to standard error, which the MCP client files with its own logs.
- The audit log records every tool call: tool, target, outcome, duration and the number of
  writes really sent to Parfumo. It is kept 90 days and read with the `audit` subcommand,
  which needs no account and changes nothing:

  ```sh
  parfumo-mcp audit                      # the 50 latest calls
  parfumo-mcp audit --writes --since 7d  # only what was really written to Parfumo
  parfumo-mcp audit --errors             # calls that ended in an error
  parfumo-mcp audit --stats              # per-tool counters: calls, errors, writes, duration
  ```

  `--tool <name>`, `--limit <n>` and `--json` narrow or reformat the output.
- There is no canary and no `/healthz` in this binary. `parfumo_session_status` tells
  whether the account is linked, whether the session is valid and what is left of the day's
  quotas.

## Platform support

The test suite passes on Linux, macOS and Windows in CI, and the released programs start on
all three. Real use, against parfumo.com, has only happened on Linux. On Windows the
database goes to `%LOCALAPPDATA%` and the local files rely on your user profile for
protection: there is no mode 600 there.
