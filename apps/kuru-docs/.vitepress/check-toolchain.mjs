import { execFileSync } from "node:child_process";

function output(command, args) {
  return execFileSync(command, args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    timeout: 30_000,
  }).trim();
}

function requireVersion(name, actual, expected) {
  if (actual !== expected) {
    throw new Error(`Expected ${name} ${expected}, selected ${actual}`);
  }
  console.log(`${name} ${actual}`);
}

// Query the app's configured pins independently of the executables on PATH.
requireVersion("Node", process.versions.node, output("mise", ["current", "node"]));
const expectedNpm = output("mise", ["current", "npm"]);
const actualNpm =
  process.platform === "win32"
    ? output("cmd.exe", ["/d", "/s", "/c", "npm --version"])
    : output("npm", ["--version"]);
requireVersion("npm", actualNpm, expectedNpm);
