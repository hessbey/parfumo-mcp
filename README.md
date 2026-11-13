# parfumo-mcp

**English** · [Français](README.fr.md) · [Deutsch](README.de.md)

[![ci](https://github.com/hessbey/parfumo-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/hessbey/parfumo-mcp/actions/workflows/ci.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![rust 1.90+](https://img.shields.io/badge/rust-1.90%2B-orange.svg)

A [Model Context Protocol](https://modelcontextprotocol.io) server, written in Rust, that
gives an AI assistant access to [Parfumo](https://www.parfumo.com): the fragrance database,
your collection, wear log, ratings and reviews, and the community side (messages, forum,
blogs, Souk).

> **Unofficial.** This project is not affiliated with or endorsed by Parfumo. It reads the
> same pages a browser does, at one request per second. Automated access may still lead
> Parfumo to suspend the linked account — use it at your own risk.

## What you get

- **48 tools**, all prefixed `parfumo_`. The 10 database tools and the public reads (forum,
  blogs, Souk, profiles) work **without a Parfumo account**.
- **Two binaries, same tools:**

  | Binary | Transport | For |
  | --- | --- | --- |
  | `parfumo-mcp` | stdio | One person, on their own machine. Nothing to host, no OAuth |
  | `parfumo-mcp-server` | Streamable HTTP | Several people, isolated accounts, on a server behind HTTPS (Docker image) |

- **Safe by default.** Every write tool supports `dry_run` and reads the real state back.
  Anything other members can see stays in dry-run until you turn public writes on, and is
  then capped per tool and per day.
- **Auditable.** Every tool call is logged locally; `parfumo-mcp audit --writes` shows what
  was really written to your account.
- **Polite to the site.** One request per second per user, random jitter, a fixed and honest
  `User-Agent`, a 24 h cache for pages and 1 h for searches and rankings. No captcha
  solving, no Cloudflare bypass.
- **Targets `www.parfumo.com`, the English site.** The German site `www.parfumo.de` uses
  different URLs and labels and is not supported.
- **Secrets stay out of the conversation.** Credentials are typed on a login page — on your
  own computer (stdio) or on the server's `/link` page (HTTP) — are encrypted at rest with
  AES-256-GCM, and never appear in logs, errors or tool results.

## Quick start (local)

With [Node.js](https://nodejs.org) installed, there is nothing to download: `npx` fetches
the program for your system (Linux, macOS or Windows). Add it to Claude Code:

```sh
claude mcp add parfumo -- npx -y parfumo-mcp
```

Or to any client configured with a JSON file (Claude Desktop and most others):

```json
{
  "mcpServers": {
    "parfumo": { "command": "npx", "args": ["-y", "parfumo-mcp"] }
  }
}
```

That is all: the public tools work at once. The first time a tool needs your Parfumo
account, the assistant gives you the link of a login page on your own computer; or open it
yourself with `npx -y parfumo-mcp login`. Your password is typed there, never in the
conversation, and is stored encrypted on your computer.

Without Node.js, download the program from the [latest release](https://github.com/hessbey/parfumo-mcp/releases/latest) and give the
client its path instead. Details: [Local usage](docs/LOCAL.md).

To host it for several people, see [Hosting](docs/HOSTING.md): the server is published as a
Docker image, `ghcr.io/hessbey/parfumo-mcp-server`.

## Tools

| Area | Tools | Account needed |
| --- | --- | --- |
| Database | search, perfume page, reviews, statements, similar perfumes, brand, perfumer, note, rankings, page fetch | No |
| Collection | list, read, add, move, update, remove, custom lists, CSV export, statistics | Yes |
| Wear log | log, list, delete | Yes |
| Ratings and reviews | rate, classify, write or delete a review or statement, list your own | Yes |
| Social | member profiles, messages, notifications, follows, wall, forum, blogs, Souk, communities | Reads: mostly no. Writes: yes |
| Profile and account | profile, settings, session status | Yes |

Full list: [Tool catalog](docs/TOOL_CATALOG.md).

## A note on text

Parfumo forbids AI-generated text in reviews, statements, forum posts and messages, and
restricts or deletes accounts that publish it. The tools that send text tell the assistant
so: it should pass on **your** words, not write them for you.

## Documentation

| Guide | Contents |
| --- | --- |
| [Local usage](docs/LOCAL.md) | The stdio binary: account, client setup, local files |
| [Hosting](docs/HOSTING.md) | The HTTP server: Docker, users, OAuth, linking a Parfumo account |
| [Configuration](docs/CONFIGURATION.md) | Every environment variable; public-write lock, caps and recipients |
| [Tool catalog](docs/TOOL_CATALOG.md) | The 48 tools, conventions, error codes |
| [Project status](docs/STATUS.md) | What is proven against the real site, what is not, what is left to do |
| [All documents](docs/README.md) | Index, including the in-depth references written in French |

## How it is built

```
crates/parfumo-client      library: access to the site. Transport, routes and selectors
                           (one module, src/site.rs), HTML parsing, models
crates/parfumo-mcp-core    library: the 48 tools, Parfumo session, storage, audit log
crates/parfumo-mcp         local binary: the tools over stdio
crates/parfumo-mcp-server  hosted binary: the tools over HTTP, with OAuth, users, canary
npm/                       the npm packages that deliver the local binary (`npx parfumo-mcp`)
```

Parfumo is a server-rendered site with no public API, so the client parses HTML. All routes
and selectors live in one module, the parsers are tested against recorded fixtures, and a
daily read-only canary on the hosted server reports any page whose structure has changed.

## Development

```sh
cargo test --workspace     # no network: recorded fixtures and local fake servers
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI runs all three on Linux, the tests again on macOS and Windows, builds the two release
binaries, then builds the Docker image and checks its size, startup and idle memory. Pushing
a tag `vX.Y.Z` publishes a release: binaries for five targets and the Docker image.

## License

[MIT](LICENSE)
