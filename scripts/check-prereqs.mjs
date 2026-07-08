#!/usr/bin/env node
// Preflight check run before `tauri dev` / `tauri build`.
// dooni's backend is Rust, so it needs the Rust toolchain (cargo/rustc).
// Without this, Tauri fails with a cryptic:
//   failed to run command `cargo metadata ...`: No such file or directory (os error 2)
// This turns that into an actionable message.

import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir, EOL } from "node:os";
import { join } from "node:path";

function has(cmd) {
  try {
    execFileSync(cmd, ["--version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

if (has("cargo") && has("rustc")) {
  process.exit(0);
}

// cargo may be installed but just missing from PATH in this shell.
const cargoBin = join(homedir(), ".cargo", "bin", "cargo");
const installedButNotOnPath = existsSync(cargoBin);

const RED = "\x1b[31m";
const YELLOW = "\x1b[33m";
const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";

const lines = [
  "",
  `${RED}${BOLD}✗ dooni can't build: the Rust toolchain (cargo) was not found.${RESET}`,
  "",
  "dooni's backend is written in Rust, so `cargo` must be installed and on your PATH.",
  "",
];

if (installedButNotOnPath) {
  lines.push(
    `${YELLOW}Rust looks installed at ~/.cargo, but it isn't on this shell's PATH.${RESET}`,
    "Load it into the current shell, then retry:",
    "",
    `  ${BOLD}source "$HOME/.cargo/env"${RESET}`,
    "",
    "(New terminals pick this up automatically.)",
  );
} else {
  lines.push(
    `${YELLOW}Install Rust (via rustup), then retry:${RESET}`,
    "",
    `  ${BOLD}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${RESET}`,
    `  ${BOLD}source "$HOME/.cargo/env"${RESET}`,
    "",
    "On macOS you also need the Xcode command line tools:",
    "",
    `  ${BOLD}xcode-select --install${RESET}`,
  );
}

lines.push("");
process.stderr.write(lines.join(EOL) + EOL);
process.exit(1);
