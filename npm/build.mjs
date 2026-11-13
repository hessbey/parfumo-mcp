// Builds the npm packages of the local program: one package per system, holding the
// program, and the `@hessbey/parfumo-mcp` package that starts the one npm installed.
//
// Every package lives under the npm account of the project (`@hessbey/…`): npm refused the
// plain name `parfumo-mcp-win32-x64` as spam, and names under an account are not checked
// that way. The plain name `parfumo-mcp` is built as well, as a copy of the main package,
// so that nobody else can take it; publishing it is allowed to fail.
//
//   node npm/build.mjs --version 0.2.1 --out npm-dist --binary <target>=<file> [...] [--all]
//
// `<target>` is the Rust target the file was built for. `--all` refuses to go on unless a
// program was given for every supported system (what a release does).

import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** Rust target → the npm package of that system. */
export const TARGETS = {
  "x86_64-unknown-linux-gnu": { os: "linux", cpu: "x64", libc: "glibc", label: "Linux x64 (glibc)" },
  "aarch64-unknown-linux-gnu": { os: "linux", cpu: "arm64", libc: "glibc", label: "Linux arm64 (glibc)" },
  "aarch64-apple-darwin": { os: "darwin", cpu: "arm64", label: "macOS, Apple silicon" },
  "x86_64-apple-darwin": { os: "darwin", cpu: "x64", label: "macOS, Intel" },
  "x86_64-pc-windows-msvc": { os: "win32", cpu: "x64", label: "Windows x64" },
};

/** The npm account every package is published under. */
export const SCOPE = "@hessbey";

/** Directory of a system's package in the output, and its name on npm. */
const directory = (target) => `parfumo-mcp-${TARGETS[target].os}-${TARGETS[target].cpu}`;
export const packageName = (target) => `${SCOPE}/${directory(target)}`;

const here = dirname(fileURLToPath(import.meta.url));
const root = dirname(here);

/**
 * Writes the packages into `out` and returns their directories: those of the systems, then
 * the main one (`parfumo-mcp`), then its copy under the plain name (`unscoped`).
 */
export function build({ version, out, binaries, all = false }) {
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) throw new Error(`not a version: ${version}`);
  for (const target of Object.keys(binaries)) {
    if (!TARGETS[target]) throw new Error(`unknown target ${target}; known: ${Object.keys(TARGETS).join(", ")}`);
    if (!existsSync(binaries[target])) throw new Error(`no file ${binaries[target]} for ${target}`);
  }
  const missing = Object.keys(TARGETS).filter((target) => !binaries[target]);
  if (all && missing.length > 0) throw new Error(`no program for: ${missing.join(", ")}`);

  const main = JSON.parse(readFileSync(join(here, "parfumo-mcp", "package.json"), "utf8"));
  rmSync(out, { recursive: true, force: true });
  const dirs = [];
  for (const [target, file] of Object.entries(binaries)) {
    const { os, cpu, libc, label } = TARGETS[target];
    const dir = join(out, directory(target));
    mkdirSync(join(dir, "bin"), { recursive: true });
    const program = join(dir, "bin", os === "win32" ? "parfumo-mcp.exe" : "parfumo-mcp");
    copyFileSync(file, program);
    chmodSync(program, 0o755);
    copyFileSync(join(root, "LICENSE"), join(dir, "LICENSE"));
    writeFileSync(
      join(dir, "README.md"),
      `# ${packageName(target)}\n\nThe \`parfumo-mcp\` program for ${label}. It is installed by the ` +
        `[\`${SCOPE}/parfumo-mcp\`](https://www.npmjs.com/package/${SCOPE}/parfumo-mcp) package: install that one.\n`
    );
    const manifest = {
      name: packageName(target),
      version,
      description: `The parfumo-mcp program for ${label}. Installed by the ${SCOPE}/parfumo-mcp package.`,
      license: main.license,
      homepage: main.homepage,
      repository: main.repository,
      os: [os],
      cpu: [cpu],
      ...(libc ? { libc: [libc] } : {}),
      files: ["bin"],
    };
    writeFileSync(join(dir, "package.json"), JSON.stringify(manifest, null, 2) + "\n");
    dirs.push(dir);
  }

  // The main package always names every system: npm installs the one that fits.
  const dir = join(out, "parfumo-mcp");
  cpSync(join(here, "parfumo-mcp"), dir, { recursive: true });
  copyFileSync(join(root, "LICENSE"), join(dir, "LICENSE"));
  main.version = version;
  main.optionalDependencies = Object.fromEntries(Object.keys(TARGETS).map((target) => [packageName(target), version]));
  writeFileSync(join(dir, "package.json"), JSON.stringify(main, null, 2) + "\n");
  dirs.push(dir);

  // The same package under the plain name.
  const plain = join(out, "unscoped");
  cpSync(dir, plain, { recursive: true });
  writeFileSync(join(plain, "package.json"), JSON.stringify({ ...main, name: "parfumo-mcp" }, null, 2) + "\n");
  dirs.push(plain);
  return dirs;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const options = { binaries: {}, all: false };
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--all") options.all = true;
    else if (args[i] === "--version") options.version = args[++i];
    else if (args[i] === "--out") options.out = args[++i];
    else if (args[i] === "--binary") {
      const [target, ...file] = (args[++i] ?? "").split("=");
      options.binaries[target] = file.join("=");
    } else throw new Error(`unknown argument ${args[i]}`);
  }
  if (!options.version || !options.out) throw new Error("usage: build.mjs --version X.Y.Z --out DIR --binary TARGET=FILE [...] [--all]");
  for (const dir of build(options)) console.log(dir);
}
