# Hosting (HTTP)

[← Documentation](README.md)

For several people: a remote server, one Docker image on a VPS, HTTPS by Caddy. The image
contains only the HTTP server, `parfumo-mcp-server`.

| | |
| --- | --- |
| Transport | MCP Streamable HTTP on `/mcp`, stateless, `rmcp` 3.9.0. Specification `2026-07-28`, and `2025-11-25` for older clients |
| Client authentication | One access key per user. It is sent as a bearer token, or typed once on the login page of an OAuth 2.1 client (PKCE S256, protected-resource metadata). No anonymous request on `/mcp` |
| Parfumo account | Linked per user on the `/link` page, in credentials mode (AES-256-GCM encrypted) or cookie mode |
| Users | Created on the command line. Cookies, private cache, quotas and audit log are isolated per user |
| Canary | Once a day, a read-only pass over every kind of page on the real site; reports any structure that changed |
| Health | `GET /healthz`, with the last canary verdict |

> `docker compose up` behind Caddy on a real VPS has not been exercised yet. CI builds the
> image and checks that it starts, and the local trial below has been run. See
> [Project status](STATUS.md).

## Install

You need a VPS with Docker and Docker Compose, and a domain name pointing to it. Caddy
obtains the HTTPS certificate by itself.

```sh
git clone https://github.com/hessbey/parfumo-mcp && cd parfumo-mcp
cp .env.example .env

# 1. Set PARFUMO_MCP_DOMAIN in .env

# 2. Generate the master key and paste it into .env (PARFUMO_MCP_MASTER_KEY=…)
docker compose run --rm --no-deps server keygen

# 3. Start
docker compose up -d
curl https://<your-domain>/healthz        # {"canary":null,"status":"ok","version":"0.1.1"}
```

Nothing is compiled on the server: Compose downloads the published image,
`ghcr.io/hessbey/parfumo-mcp-server`, built for amd64 and arm64.

- **Version.** Without a setting, the latest release runs. To pin one, add
  `PARFUMO_MCP_VERSION=0.1.1` to `.env`: an upgrade then only happens when you change it.
- **Upgrade.** `docker compose pull`, then `docker compose up -d`.
- **Your own build.** `docker build -t ghcr.io/hessbey/parfumo-mcp-server:local .`, then
  `PARFUMO_MCP_VERSION=local` in `.env`.

Open ports 80 and 443 on the VPS, and only those: the server's port 8080 does not leave the
Docker network. Then [add a user](#add-a-user).

**Back up the master key.** Without it, stored Parfumo credentials and cookies are
unreadable and every user must link their account again.

The container runs unprivileged: user 10001, read-only file system outside the `/data`
volume, no Linux capability, 256 MB of memory at most. CI builds the image from
every commit and checks its size, its non-root user, its startup and its idle memory;
`scripts/check-image.sh` runs the same checks on an image you build yourself.

### Local trial, without HTTPS or a domain

This one builds the image from the source tree, about ten minutes the first time, so that
a change can be tried before it is released. It needs a `.env` holding the master key.

```sh
docker compose -f docker-compose.local.yml up -d
curl http://localhost:8080/healthz
```

### Without Docker

```sh
cargo build --release -p parfumo-mcp-server
export PARFUMO_MCP_MASTER_KEY=$(./target/release/parfumo-mcp-server keygen)
export PARFUMO_MCP_DB=./data/parfumo-mcp.db PARFUMO_MCP_PUBLIC_URL=http://localhost:8080
./target/release/parfumo-mcp-server serve
```

All settings: [Configuration](CONFIGURATION.md).

## Add a user

There is no open sign-up. The administrator creates each user on the command line:

```sh
docker compose exec server parfumo-mcp-server user add alice
```

The command prints the user's **access key** (`pmcp_…`) **once**. It is the only thing to
hand over, through a safe channel. The user needs nothing else: no name, no password. The
key is 256 random bits, and the database keeps only its SHA-256 hash. The user pastes it:

- on the `/link` page, to link a Parfumo account;
- on the login page an OAuth client opens;
- or in an `Authorization: Bearer` header, for clients that only handle a fixed header.

Other commands — in Docker, prefix each with `docker compose exec server`:

```sh
parfumo-mcp-server user list                    # users and the state of their Parfumo link
parfumo-mcp-server user rotate-key <name>       # new access key, OAuth sessions revoked
parfumo-mcp-server user remove <name>           # erase the user, their secrets and sessions
parfumo-mcp-server audit --user <name>          # tool-call log (kept 90 days)
parfumo-mcp-server audit --writes --since 7d    # only what was really written to Parfumo
parfumo-mcp-server audit --stats --since 24h    # per-tool counters: calls, errors, writes, duration
parfumo-mcp-server canary                       # run the canary now; --last: last report
```

The access key does not expire: if it is lost or leaked, replace it with `user rotate-key`,
which refuses the old key at once and signs the user out of every OAuth client. Twenty wrong
keys in fifteen minutes lock the address they come from.

## Connect an MCP client

The server address is `https://<your-domain>/mcp`.

**OAuth-capable client** (Claude, MCP Inspector…): give it the URL only. The client
discovers the authorization server through `/.well-known/oauth-protected-resource`, opens
the server's login page in the browser, and the user pastes their access key.
The client identifies itself with a Client ID Metadata Document (an `https` URL) or, for
older clients, by dynamic registration. The login page shows the address the client will be
sent back to: authorize only if you started the connection yourself.

**Fixed-header client**: add the header `Authorization: Bearer pmcp_…`. With Claude Code:

```sh
claude mcp add --transport http parfumo https://<your-domain>/mcp \
  --header "Authorization: Bearer pmcp_…"
```

Check with MCP Inspector on the command line:

```sh
npx @modelcontextprotocol/inspector --cli https://<your-domain>/mcp --transport http \
  --header "Authorization: Bearer pmcp_…" --method tools/list
```

## Link a Parfumo account

The database tools and the public reads (forum, blogs, Souk, communities, profiles) work
without a Parfumo account and never send the account's cookies. The other tools act on the
linked account; without a link they return `not_linked`.

Each user opens `https://<your-domain>/link`, in English, French or German, and pastes their
access key. The page asks for the Parfumo login; the other choices are under "More options":

- **Credentials mode** (the main form): Parfumo nickname and password, encrypted on the server. When the
  session expires, the server logs in again by itself: three attempts at most, then
  `reauth_required`. A rejected password or a human check stops it at once.
- **Cookie mode**: the user pastes the `Cookie` header of their logged-in browser; the only
  cookie needed is `pf_auth`. No password on the server. When it expires, tools return
  `reauth_required`. Clicking "Logout" in that browser revokes the cookie.
- **Test** the link, or **unlink** the account (credentials and cookies erased).

If Parfumo rejects the stored password, the server stops trying for that account until it is
linked again on this page: rejected credentials are never replayed. The server never solves
a captcha: if Parfumo shows one, the tool returns `captcha_required` and the way out is
cookie mode.

Nothing is ever typed into the conversation with the assistant or passed as a tool
parameter. Automated access may lead Parfumo to suspend the linked account; the page says so
and asks for confirmation.

## Operations

The canary, what to do when the site changes, audit log, backup and upgrade are covered in
[MAINTENANCE.md](MAINTENANCE.md) (in French).
