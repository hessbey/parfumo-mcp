# parfumo-mcp

Unofficial [MCP](https://modelcontextprotocol.io) server for [Parfumo](https://www.parfumo.com):
48 tools over the fragrance database, your collection, wear log, ratings, reviews and the
community, for AI assistants. Not affiliated with Parfumo.

This package installs the local program for your system: Linux (x64, arm64, glibc 2.35 or
newer), macOS (Apple silicon, Intel) and Windows (x64).

## Use

Claude Code:

```sh
claude mcp add parfumo -- npx -y parfumo-mcp
```

Any client configured with a JSON file (Claude Desktop and most others):

```json
{
  "mcpServers": {
    "parfumo": { "command": "npx", "args": ["-y", "parfumo-mcp"] }
  }
}
```

The public tools work at once. The first time a tool needs your Parfumo account, the
assistant gives you the link of a login page on your own computer; or open it yourself:

```sh
npx -y parfumo-mcp login
```

Your password is typed there, never in the conversation, and is stored encrypted on your
computer.

Documentation, source and the hosted HTTP server:
<https://github.com/hessbey/parfumo-mcp>
