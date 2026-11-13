// Checks the npm packages for real: builds them around a program built on this machine,
// packs them, installs the archives in an empty project and runs the program through npm's
// launcher. Nothing is published and nothing is fetched from the npm registry.
//
//   node npm/test.mjs target/release/parfumo-mcp

import { execFileSync, spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { SCOPE, TARGETS, build, packageName } from "./build.mjs";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const binary = resolve(process.argv[2] ?? "");
if (!process.argv[2] || !existsSync(binary)) throw new Error("usage: test.mjs <path of a parfumo-mcp program built for this machine>");

const target = Object.keys(TARGETS).find((t) => TARGETS[t].os === process.platform && TARGETS[t].cpu === process.arch);
if (!target) throw new Error(`no npm package is made for ${process.platform}-${process.arch}`);
const version = readFileSync(join(root, "Cargo.toml"), "utf8").match(/^version = "([^"]+)"/m)[1];

const work = mkdtempSync(join(tmpdir(), "parfumo-mcp-npm-"));
const check = (ok, what, detail = "") => {
  if (!ok) throw new Error(`${what}${detail ? `\n${detail}` : ""}`);
  console.log(`ok  ${what}`);
};
// npm is a script on Windows, which only a shell can start.
const npm = (args, cwd) => execFileSync("npm", args, { cwd, encoding: "utf8", shell: process.platform === "win32", stdio: ["ignore", "pipe", "inherit"] });

try {
  const [platformDir, mainDir, plainDir] = build({ version, out: join(work, "packages"), binaries: { [target]: binary } });
  const manifest = JSON.parse(readFileSync(join(mainDir, "package.json"), "utf8"));
  check(manifest.version === version && manifest.name === `${SCOPE}/parfumo-mcp`, `${SCOPE}/parfumo-mcp carries the version of Cargo.toml (${version})`);
  check(
    Object.keys(manifest.optionalDependencies).length === Object.keys(TARGETS).length &&
      Object.entries(manifest.optionalDependencies).every(([name, v]) => name.startsWith(`${SCOPE}/`) && v === version),
    "the main package names the program of every system, at the same version"
  );

  // Paths given to npm are relative: the temporary directory may hold a space.
  const pack = (dir) => npm(["pack", "--loglevel=error", "--pack-destination", "../.."], dir).trim().split("\n").pop();
  const [platformTgz, mainTgz, plainTgz] = [pack(platformDir), pack(mainDir), pack(plainDir)];
  const files = JSON.parse(npm(["pack", "--loglevel=error", "--dry-run", "--json"], platformDir))[0].files.map((f) => f.path);
  check(files.some((f) => f.startsWith("bin/parfumo-mcp")) && files.includes("LICENSE"), `${packageName(target)} holds the program and the licence`);

  // The programs of the other systems are not published yet: they are left out, as npm
  // itself leaves out those that do not fit the machine.
  const install = (name, archives) => {
    const dir = join(work, name);
    mkdirSync(dir);
    writeFileSync(join(dir, "package.json"), JSON.stringify({ name, private: true }));
    npm(["install", "--omit=optional", "--no-audit", "--no-fund", "--loglevel=error", ...archives.map((a) => `../${a}`)], dir);
    return dir;
  };
  const project = install("with-program", [mainTgz, platformTgz]);
  const launcher = join(project, "node_modules", SCOPE, "parfumo-mcp", "bin", "parfumo-mcp.js");
  check(existsSync(join(project, "node_modules", ".bin", process.platform === "win32" ? "parfumo-mcp.cmd" : "parfumo-mcp")), "npm creates the parfumo-mcp command");

  const env = { ...process.env, PARFUMO_MCP_DB: join(work, "data", "p.db"), PARFUMO_BASE_URL: "http://127.0.0.1:9" };
  mkdirSync(join(work, "data"));
  const run = (args) => spawnSync(process.execPath, [launcher, ...args], { encoding: "utf8", env });

  const shown = run(["--version"]);
  check(shown.status === 0 && shown.stdout.trim() === `parfumo-mcp ${version}`, "the command starts the program", shown.stdout + shown.stderr);
  const refused = run(["audit", "--since", "soon"]);
  check(refused.status === 1 && refused.stdout === "" && refused.stderr.includes("error:"), "an error keeps its exit code and its message", refused.stderr);

  // The MCP protocol goes through untouched, and closing the input ends everything.
  const answer = await new Promise((done, fail) => {
    const child = spawn(process.execPath, [launcher], { env, stdio: ["pipe", "pipe", "ignore"] });
    let out = "";
    const timer = setTimeout(() => (child.kill(), fail(new Error("no answer to initialize"))), 30000);
    child.stdout.on("data", (chunk) => {
      out += chunk;
      if (out.includes("\n")) child.stdin.end();
    });
    child.on("exit", (code) => (clearTimeout(timer), done({ code, line: out.split("\n")[0] })));
    child.stdin.write(
      JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-11-25", capabilities: {}, clientInfo: { name: "npm-test", version: "0" } } }) + "\n"
    );
  });
  const info = JSON.parse(answer.line).result.serverInfo;
  check(info.name === "parfumo-mcp" && info.version === version, "an MCP client talks to the program through the command", answer.line);
  check(answer.code === 0, "closing the input ends the command cleanly");

  // Without the program of this system, the command says so instead of failing obscurely.
  const bare = install("without-program", [mainTgz]);
  const lost = spawnSync(process.execPath, [join(bare, "node_modules", SCOPE, "parfumo-mcp", "bin", "parfumo-mcp.js"), "--version"], { encoding: "utf8" });
  check(
    lost.status === 1 && lost.stdout === "" && lost.stderr.includes(`${process.platform}-${process.arch}`) && lost.stderr.includes("/releases"),
    "a missing program is reported with the place to download one",
    lost.stderr
  );

  // The copy under the plain name starts the same program.
  const plain = install("plain-name", [plainTgz, platformTgz]);
  const same = spawnSync(process.execPath, [join(plain, "node_modules", "parfumo-mcp", "bin", "parfumo-mcp.js"), "--version"], { encoding: "utf8" });
  check(same.status === 0 && same.stdout.trim() === `parfumo-mcp ${version}`, "the plain name parfumo-mcp starts the same program", same.stdout + same.stderr);
  console.log(`\nnpm packages of parfumo-mcp ${version} for ${target}: all checks passed`);
} finally {
  // Windows keeps a program's file locked for a moment after it ends.
  rmSync(work, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
}
