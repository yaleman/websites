import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";

const SCRIPT_PATH = path.resolve("scripts/sync-pnpm-overrides.mjs");

function runNodeScript(args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, args, options);
    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });
    child.on("error", reject);
    child.on("close", (code) => {
      resolve({ code, stdout, stderr });
    });
  });
}

test("sync-pnpm-overrides copies lockfile overrides into package.json", async () => {
  const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "sync-pnpm-overrides-"));

  await fs.writeFile(
    path.join(tempDir, "package.json"),
    JSON.stringify(
      {
        name: "fixture",
        version: "1.0.0",
        packageManager: "pnpm@10.30.3",
      },
      null,
      2,
    ) + "\n",
  );

  await fs.writeFile(
    path.join(tempDir, "pnpm-lock.yaml"),
    [
      "lockfileVersion: '9.0'",
      "",
      "settings:",
      "  autoInstallPeers: true",
      "  excludeLinksFromLockfile: false",
      "",
      "overrides:",
      "  uuid@<11.1.1: '>=11.1.1'",
      "  webpack-dev-server@<=5.2.3: '>=5.2.4'",
      "",
      "importers:",
      "",
      "  .: {}",
      "",
    ].join("\n"),
  );

  const result = await runNodeScript([SCRIPT_PATH, tempDir], {
    cwd: tempDir,
    env: process.env,
  });

  assert.equal(result.code, 0, `expected success, got stderr: ${result.stderr}`);

  const packageJson = JSON.parse(
    await fs.readFile(path.join(tempDir, "package.json"), "utf8"),
  );

  assert.deepEqual(packageJson.pnpm?.overrides, {
    "uuid@<11.1.1": ">=11.1.1",
    "webpack-dev-server@<=5.2.3": ">=5.2.4",
  });
});

test("sync-pnpm-overrides leaves package.json unchanged when lockfile has no overrides", async () => {
  const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "sync-pnpm-overrides-"));
  const packageJsonPath = path.join(tempDir, "package.json");
  const originalPackageJson =
    JSON.stringify(
      {
        name: "fixture",
        version: "1.0.0",
        packageManager: "pnpm@10.30.3",
      },
      null,
      2,
    ) + "\n";

  await fs.writeFile(packageJsonPath, originalPackageJson);
  await fs.writeFile(
    path.join(tempDir, "pnpm-lock.yaml"),
    [
      "lockfileVersion: '9.0'",
      "",
      "settings:",
      "  autoInstallPeers: true",
      "  excludeLinksFromLockfile: false",
      "",
      "importers:",
      "",
      "  .: {}",
      "",
    ].join("\n"),
  );

  const result = await runNodeScript([SCRIPT_PATH, tempDir], {
    cwd: tempDir,
    env: process.env,
  });

  assert.equal(result.code, 0, `expected success, got stderr: ${result.stderr}`);
  assert.match(result.stdout, /No top-level overrides found/);
  assert.equal(await fs.readFile(packageJsonPath, "utf8"), originalPackageJson);
});
