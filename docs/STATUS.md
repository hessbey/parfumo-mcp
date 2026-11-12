# Project status

[← Documentation](README.md)

Version 0.1.1. All five milestones of the [specification](SPECIFICATION.md) are
delivered, plus the local stdio binary. This page says what has been proven against the real
site and what has not. Nothing here is assumed: a route that was never exercised is listed
as such. The dated, detailed record is [MILESTONES.md](MILESTONES.md) (in French).

> **Most live evidence predates the last restructuring.** The rows marked "live" below
> were measured on 11 and 12 November 2026, before the code was split into four crates and
> the binaries renamed. On the current code, the live evidence is the canary: all 48
> read-only checks passed on the real site (see the Canary row). The write tools have not
> been sent live again since; the offline test suite and CI cover them.

## Proven

| Area | Evidence |
| --- | --- |
| Tests | 202 tests, no network access: parsers on recorded fixtures, the tools called in process against a fake site, the HTTP server (OAuth, tokens, isolation, canary) on its own, the real local binary driven through its pipes. `clippy -D warnings` is clean. Both run in CI on every push, the tests on Linux, macOS and Windows |
| Docker image | Built in CI; `scripts/check-image.sh` fails the build over 50 MB, if it runs as root, if `/healthz` does not answer, or over 100 MB of idle memory |
| Database tools | Each of the 10 called on the live site through the server; outputs in [TOOLS.md](TOOLS.md) |
| Login and session | Credentials mode and cookie mode, live, through the HTTP server; cookie mode live through the stdio binary |
| Collection | Add, move, update, remove, custom lists, export and statistics, live on a test account, each state read back |
| Ratings and classification | Sent live on one perfume, read back, then removed |
| Social reads | All 13 read tools live: profiles, inbox, notifications, follows, wall, forum, blogs, Souk, communities, settings |
| Some public writes | Settings, signature scent, time zone, follow and unfollow: sent live, read back, restored |
| Safeguards | Public-write lock, daily caps (persisting across restarts) and recipient allow-list: by test and live |
| Canary | 48 read-only checks `ok` on the live site (27 public, 21 on the test account), no write. Run again on the current code on 12 November 2026 by the maintainer, in the Docker image started with `docker-compose.local.yml`, the test account linked in credentials mode through the `/link` page in a browser: 48 ok, 0 changed, 0 failed, 0 skipped |
| Web pages in a browser | The `/link` page and the whole OAuth flow (login page, authorization code, token, then `/mcp`) driven with a real Chromium against the release server, on 12 November 2026: access key alone, language switch, wrong key refused. An earlier pass caught a bug the tests had missed: both forms were refused by every browser |
| Isolation and secrets | Two users never see each other's data or actions; no secret in logs, errors, tool results or the canary report — by test |
| Security audit | Nine findings, all fixed, seven with a dedicated test: [AUDIT.md](AUDIT.md) |
| Footprint | 20 MB resident memory at rest; about 3 ms of overhead per call; binaries of about 10 MB |

## Not proven

**Text was never published.** Sending a review, a statement, a private message, a wall post,
a forum post or a blog article needs a text, and Parfumo forbids AI-generated text. None was
written for the occasion. These tools are proven up to the payload — target resolved, form
and token read on the live site, request built — and no further. Lifting this needs texts
written by the account holder.

Other points:

- **Wear log.** Logging and deleting a wear has only run against the fake site: a wear shows
  on the public profile.
- **Souk.** No entry was ever published: a made-up request would be fake content shown to
  other members, and offers are refused to recent accounts.
- **Non-empty states.** A conversation, a message and a notification have never been seen
  live through the tools; the test account has none.
- **Rich text.** Forum and blog editors produce rich text; the tools send plain text and the
  rendering is unverified. The blog form is "Save & Preview": the publishing step is unknown.
- **Real session expiry.** Automatic re-login has only run against the fake site; no session
  expired during the trials.
- **Deployment.** The image has run locally with `docker-compose.local.yml`, but
  `docker compose up` behind Caddy on a real VPS has not: nor the per-address login lock
  behind a real reverse proxy, the daily canary trigger, or Cloudflare's behaviour from a
  VPS address.
- **Real MCP clients.** Only MCP Inspector on the command line has been used. The OAuth
  flow has run in a real browser with a scripted client, not with a real MCP client, and no
  desktop client has launched the stdio binary.
- **Platforms.** The test suite passes on Linux, macOS and Windows in CI, and the release
  programs start on all three. Outside Linux, nothing has been run against the real site
  or by a real MCP client.
- **Smaller gaps.** Five search sort orders, three of the five rating scales, lists longer
  than one page, custom-column values read back, gender and country updates.

## Not implemented

- Removing a Souk entry.
- Community writes (joining, commenting).
- The activity feed of followed members.
- Editing a blog article; avatar upload.
- MCP prompts and resources (optional in the specification).
- Contributions to the database through Parfumo Research (planned for a version 2).

## Known limits

- The first call of `parfumo_get_similar_perfumes` for a perfume takes about ten seconds
  (some ten requests at one per second); the cache then serves it for 24 h.
- The one-request-per-second limit is shared between the public tools and the account
  session of the same user.
- Only `www.parfumo.com` in English is supported. The German site `www.parfumo.de` serves
  the same pages under German URLs and labels: routes and parsers would need a second set.
  It was seen during exploration, never tested.
- There is no general "top" ranking and no search filter by concentration: the site has
  neither.
- The access key of a user does not expire; replace it with `user rotate-key`.
- Requests without an `Origin` header are accepted on `/mcp`, as every non-browser MCP
  client sends none.
