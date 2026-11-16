# Project status

[← Documentation](README.md)

Version 0.3.0. All five milestones of the [specification](SPECIFICATION.md) are
delivered, plus the local stdio binary. This page says what has been proven against the real
site and what has not. Nothing here is assumed: a route that was never exercised is listed
as such. The dated, detailed record is [MILESTONES.md](MILESTONES.md) (in French).

> **Most live evidence predates the last restructuring.** The rows marked "live" below
> were measured on 11 and 12 November 2026, before the code was split into four crates and
> the binaries renamed. Since then, the live evidence is the canary run of 12 November 2026
> (all 48 read-only checks of that version passed on the real site, see the Canary row) and,
> for what version 0.3.0 adds, the read-only check of 15 November 2026 (see the Application
> interface row). The write tools have not been sent live again since; the offline test
> suite and CI cover them.

## Proven

| Area | Evidence |
| --- | --- |
| Tests | 382 tests, no network access: parsers on recorded fixtures, the reads of the application interface on its recorded answers, the tools called in process against a fake site, the HTTP server (OAuth, tokens, isolation, canary) on its own, the real local binary driven through its pipes. `clippy -D warnings` is clean. Both run in CI on every push, the tests on Linux, macOS and Windows |
| Docker image | Built in CI; `scripts/check-image.sh` fails the build over 50 MB, if it runs as root, if `/healthz` does not answer, or over 100 MB of idle memory |
| Database tools | Each of the 10 called on the live site through the server; outputs in [TOOLS.md](TOOLS.md) |
| Application interface (0.3.0) | For an account linked in credentials mode, part of the database reads go through the interface of Parfumo's own application ([APP_API.md](APP_API.md)). By test, no network access: every read the code makes through it (search, perfume record, brand, perfumer, release-year listing, similar perfumes and layering, the member's own nickname, ratings and classification votes) against answers recorded on the real site, served by a fake site; the hand-over to the web site (a lookup that finds nothing, a listing without its count, an error object, a refused token, the block page); the web read that completes an answer, and its warning when it fails; the session rules (a refused token forgotten for every client of the user and in the database, one login at a time, the pauses, refused credentials never sent again; the refused token, the refused credentials and the pause after a renewed session is lost hold across a restart; two processes on one database, such as the canary command beside the server, neither send a token the other saw refused nor erase the token the other renewed); the similar perfumes of a perfume the interface no longer has read on the web site; the canary stopping when the interface asks to wait or a completing page shows a captcha; the token never sent to a web page and never found in logs, audit rows, tool results or the canary report; `parfumo_fetch_page` refusing any address under `/app/`; a 0.2 database upgraded in place. Live, read-only, on 15 November 2026 with the test account, `GET` requests only: search, perfume record, brand, perfumer and release-year listing read through the interface; the same perfumes in the same order as the web site for a text search, a search by year, pages 1 and 2 of a brand, a perfumer page and a release-year listing; the rank label and the country equal to the web answer after the completing read; all twelve sort codes of the search honoured; a search by year answered beyond its thousandth result; no request to the web site carried the session token. Run again on 16 November 2026 on the final code, with the same results, no token refused and no request told to wait. Similar perfumes and the member's own nickname, ratings and classification votes were read through the interface that day and answered, without a comparison with the web site (its reads are `POST` requests or panels of the account) |
| Login and session | Credentials mode and cookie mode, live, through the HTTP server; cookie mode live through the stdio binary |
| Collection | Add, move, update, remove, custom lists, export and statistics, live on a test account, each state read back |
| Ratings and classification | Sent live on one perfume, read back, then removed |
| Social reads | All 13 read tools live: profiles, inbox, notifications, follows, wall, forum, blogs, Souk, communities, settings |
| Some public writes | Settings, signature scent, time zone, follow and unfollow: sent live, read back, restored |
| Safeguards | Public-write lock, daily caps (persisting across restarts) and recipient allow-list: by test and live |
| Canary | 48 read-only checks `ok` on the live site (27 public, 21 on the test account), no write. Run again after the restructuring, on 12 November 2026, by the maintainer, in the Docker image started with `docker-compose.local.yml`, the test account linked in credentials mode through the `/link` page in a browser: 48 ok, 0 changed, 0 failed, 0 skipped. Version 0.3.0 adds a third area, `interface`: nine checks of the reads made through the application interface, run only when the canary account is linked in credentials mode (57 checks then; a cookie-linked account gets 48 ok and 1 not applicable). That area is proven by test against a fake site only: it has never run against the real site (see below) |
| npm packages | Built around the release program, packed, installed in an empty project and run through npm's launcher, in CI on Linux, macOS and Windows: the command starts the program, an MCP client talks through it, exit codes and errors pass, a missing program is reported. Published since 0.2.2, with provenance: `npx -y parfumo-mcp`, run on Linux x64 against the real registry on 13 November 2026, installed the program and answered an MCP client with its 48 tools. The same command was run on Windows by the maintainer the same day |
| Local login page | The real local binary against a fake Parfumo, by test: an account linked on the page with no setting at all, used at once by the running server and kept across a restart; other addresses, other host names and forms from other sites refused; `login` and `logout` from another process. Driven with a real Chromium on 13 November 2026: language switch, wrong password refused, account linked, page closed. Live: the maintainer linked the Parfumo test account through the page with `npx -y parfumo-mcp login` on Windows, on 13 November 2026 |
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

**How the code handles the failures of the application interface was never exercised on the
real site**, and other points of version 0.3.0 were not measured there:

- **Canary.** The `interface` area has only run against the fake site, never against the
  real one. The last run of the canary on the real site recorded here is that of
  12 November 2026, before that area existed.
- **Refusals.** A token the interface refuses, a block of the address, and a wrong password
  at the login of the interface have never been seen live from this code. Their handling
  rests on tests. The real answer of `/auth/login` to a wrong password was never recorded:
  the code takes for a refusal only a 401 with a JSON body, or a 2xx whose JSON says
  `"status":"error"`. The block page was never recorded as a fixture: it is recognised from
  what was noted when it was met.
- **Token lifetime.** How long a session token lives is unknown.
- **`POST` reads not compared.** The live check was `GET` only. The web reads that are
  `POST` requests (reviews, statements, similar perfumes, the classification chart, the
  brand filter of the web search) were not compared with the interface: similar perfumes
  and the classification of a perfume record are read through the interface without that
  comparison.

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
- **npm.** `npx -y parfumo-mcp` has run on Linux x64 and on Windows, not on macOS, where CI
  installs the packages from local archives. Publishing without a token (trusted
  publishing) has never run: the releases so far used the token.
- **Local login page.** No real MCP client has handed its address to a user: the live trial
  went through the `login` command. It ran on Windows only.
- **Platforms.** The test suite passes on Linux, macOS and Windows in CI, and the release
  programs start on all three. Against the real site, the tools have only run on Linux; on
  Windows, only the login. Nothing has run on macOS, nor through a real MCP client.
- **Smaller gaps.** Five sort orders of the web search (the application interface was seen
  to honour all twelve on 15 November 2026), three of the five rating scales, lists longer
  than one page, custom-column values read back, gender and country updates.

## Not implemented

- Removing a Souk entry.
- Community writes (joining, commenting).
- The activity feed of followed members.
- Editing a blog article; avatar upload.
- MCP prompts and resources (optional in the specification).
- Contributions to the database through Parfumo Research (planned for a version 2).
- Through the application interface: the collection, the wear log, member profiles and
  every write. They were left out of version 0.3.0 on purpose; these tools work as before,
  through the web site.

## Known limits

- The first call of `parfumo_get_similar_perfumes` for a perfume takes about ten seconds
  (some ten requests at one per second), every time: no answer is kept.
- The one-request-per-second limit is shared between the public tools and the account
  session of the same user, web pages and application interface together.
- The application interface is only used for an account linked in credentials mode, and
  only for part of the database reads. A cookie-linked account, or no account, reads the
  web pages for everything, as before.
- A listing read through the interface gives the full name of a perfume ("Sauvage (Eau de
  Toilette)") and no separate `concentration`; the web listing gives the two apart.
- `parfumo_session_status` does not show that the interface refused the credentials of an
  account; only the server log does. The tools then read the web pages.
- Only `www.parfumo.com` in English is supported. The German site `www.parfumo.de` serves
  the same pages under German URLs and labels: routes and parsers would need a second set.
  It was seen during exploration, never tested.
- There is no general "top" ranking and no search filter by concentration: the site has
  neither.
- The access key of a user does not expire; replace it with `user rotate-key`.
- Requests without an `Origin` header are accepted on `/mcp`, as every non-browser MCP
  client sends none.
