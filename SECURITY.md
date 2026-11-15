# Security policy

This project handles Parfumo credentials and session cookies, so reports are welcome.

Since version 0.3.0 it also holds, for an account linked with its password, the session
token of the interface Parfumo's own application uses. That token is stored encrypted
together with the session cookies, per user; it is sent to that interface only, never to a
web page, and never appears in a log, an audit row, an error or a tool result. The key of
Parfumo's application itself is never used.

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private reporting instead: on this
repository, open the **Security** tab and choose **Report a vulnerability**.

A useful report says which binary is affected (`parfumo-mcp` or `parfumo-mcp-server`), the
version or commit, and the steps to reproduce. Never include a real password, session cookie,
session token or bearer token: describe where it leaks instead.

## Scope

In scope: the code of this repository. For example, a secret that reaches a log, an error
or a tool result; one user of the HTTP server reading or acting on another user's account; a
way around the OAuth flow, the login throttling or the public-write safeguards; a request to
Parfumo's application interface, other than its login, that leaves without the member's own
session token, or that token sent anywhere else.

Out of scope: vulnerabilities in parfumo.com itself. Report those to Parfumo.

## Supported versions

Only the latest commit of the `main` branch is supported.

## Known limits

The security audit, its nine fixed findings, what version 0.3.0 added to protect and the
points left open are in [docs/AUDIT.md](docs/AUDIT.md) (in French).
