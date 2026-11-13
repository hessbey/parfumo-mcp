#!/usr/bin/env node
// Starts the parfumo-mcp program that npm installed for this system, with the same
// arguments, input and output. The program itself is in a package of its own, one per
// system (@hessbey/parfumo-mcp-linux-x64, @hessbey/parfumo-mcp-win32-x64, ...), which npm
// picks by itself.
"use strict";

const { spawn } = require("node:child_process");

const system = `${process.platform}-${process.arch}`;
const releases = "https://github.com/hessbey/parfumo-mcp/releases";

function fail(reason) {
  // Standard error only: standard output belongs to the MCP protocol.
  console.error(`parfumo-mcp: ${reason}\nPrograms for every supported system: ${releases}`);
  process.exit(1);
}

let program;
try {
  program = require.resolve(`@hessbey/parfumo-mcp-${system}/bin/parfumo-mcp${process.platform === "win32" ? ".exe" : ""}`);
} catch {
  fail(
    `no program for this system (${system}) was installed. Supported: Linux with glibc (x64, arm64), ` +
      `macOS (arm64, x64) and Windows (x64). If yours is one of them, npm skipped optional ` +
      `dependencies: install again without --omit=optional or --no-optional.`
  );
}

const child = spawn(program, process.argv.slice(2), { stdio: "inherit", windowsHide: true });
child.on("error", (error) => fail(`${program} could not be started: ${error.message}`));
child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 1);
  }
});
// The MCP client stops this process; the program must stop with it.
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => child.kill(signal));
}
