# Local usage (stdio)

[← Documentation](README.md)

For one person, on their own machine. The MCP client (Claude Code, Claude Desktop…) starts
`parfumo-mcp` and talks to it over standard input and output. Nothing listens on the
network, apart from a login page on `127.0.0.1` while you link your account.

## Install

**With npm**, if [Node.js](https://nodejs.org) 18 or newer is installed, there is nothing to
install by hand: the client starts the program with `npx -y @hessbey/parfumo-mcp`, which fetches the
one for your system the first time and keeps it. To stay on a version, name it:
`npx -y @hessbey/parfumo-mcp@<version>`. The npm package holds the same programs as the archives
below, and npm shows that they were built by this repository (provenance).

**Or download.** Take the archive for your system from the [latest release](https://github.com/hessbey/parfumo-mcp/releases/latest) and unpack
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

## Connect a client

Claude Code:

```sh
claude mcp add parfumo -- npx -y @hessbey/parfumo-mcp
```

Clients configured with a JSON file (Claude Desktop and most others):

```json
{
  "mcpServers": {
    "parfumo": { "command": "npx", "args": ["-y", "@hessbey/parfumo-mcp"] }
  }
}
```

With a downloaded or built program, give its path instead of `npx`:
`claude mcp add parfumo -- /path/to/parfumo-mcp`, or `"command": "/path/to/parfumo-mcp"`
without `args`. The commands of this page are then written `parfumo-mcp …` with that path;
with npm, write `npx -y @hessbey/parfumo-mcp …`.

There is nothing else to set. The public tools (database, forum, blogs, Souk, profiles) work
at once; the tools that act on your account answer `not_linked` until you link it.

Check it with MCP Inspector, no account needed:

```sh
npx @modelcontextprotocol/inspector --cli target/release/parfumo-mcp --method tools/list
```

## Link your Parfumo account

The account is linked on a page of your own computer. Nothing is typed into the conversation
with the assistant, and nothing is written in the client's configuration.

- **From the assistant.** The first time a tool needs the account, the assistant receives
  the address of the login page, `http://127.0.0.1:<port>/<secret>`, and passes it on. Open
  it, enter your Parfumo nickname and password, tick the box, then ask again.
- **From a terminal.** `parfumo-mcp login` opens the same page in your browser
  (`--no-browser` only prints its address). A client that is already running the program
  finds the account at its next tool call, without a restart.

The page is in English, French or German. Its main form is **credentials mode**: the
program logs in, keeps the session between runs, and logs in again by itself when it expires
(three attempts at most, then `reauth_required`). Under "More options":

- **Cookie mode**, no password: paste the `pf_auth=…` value copied from a browser logged in
  to Parfumo (the whole `Cookie` header works too). When it expires, tools return
  `reauth_required` and you paste a new one. Clicking "Logout" in that browser revokes it.
- **Test** the link, or **unlink** the account. `parfumo-mcp logout` unlinks it too.

If Parfumo rejects the password, the program never sends it again — even after a restart —
until you link the account again. It never solves a captcha: if Parfumo shows one, the tool
returns `captcha_required` and the way out is cookie mode.

What the page is, and is not:

- It listens on `127.0.0.1` only, on a port the system picks, and only while it is needed:
  ten minutes, or until an account is linked. Each opening has a new address.
- The address holds a random secret. The page answers no other address, no other host name,
  and no form sent from another site; it runs no script and loads nothing from the network.
- The address goes through the conversation, since the assistant shows it. It leads to the
  form, never to what is stored: knowing it does not reveal a password or a cookie.
- It only works when the browser and the program are on the same computer. If the program
  runs elsewhere (a remote machine, a container), use the environment instead.

### Or from the environment

The account can also be given by the environment of the process, for a setup without a
browser:

| Variable | Mode |
| --- | --- |
| `PARFUMO_COOKIE` | **Cookie mode**: the `pf_auth=…` value, or the whole `Cookie` header |
| `PARFUMO_USER` and `PARFUMO_PASSWORD` | **Credentials mode** |

```sh
claude mcp add parfumo -e PARFUMO_COOKIE='pf_auth=…' -- /path/to/parfumo-mcp
```

The two modes are exclusive: with both, or with half of the credentials pair, the program
refuses to start and says why. An account of the environment takes precedence: the login
page is not offered and `parfumo-mcp login` refuses to run. Change the variables to change
the account; remove them and the account they gave is erased at the next start.

A password written in the client's configuration is stored there in clear text. That is a
reason to prefer the login page, or cookie mode.

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
- To switch accounts, link another one on the login page; `parfumo-mcp logout` erases the
  stored one.
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
