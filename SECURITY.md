# Security policy

This project handles Parfumo credentials and session cookies, so reports are welcome.

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private reporting instead: on this
repository, open the **Security** tab and choose **Report a vulnerability**.

A useful report says which binary is affected (`parfumo-mcp` or `parfumo-mcp-server`), the
version or commit, and the steps to reproduce. Never include a real password, session cookie
or bearer token: describe where it leaks instead.

## Scope

In scope: the code of this repository. For example, a secret that reaches a log, an error
or a tool result; one user of the HTTP server reading or acting on another user's account; a
way around the OAuth flow, the login throttling or the public-write safeguards.

Out of scope: vulnerabilities in parfumo.com itself. Report those to Parfumo.

## Supported versions

Only the latest commit of the `main` branch is supported.

## Known limits

The security audit, its nine fixed findings and the points left open are in
[docs/AUDIT.md](docs/AUDIT.md) (in French).
