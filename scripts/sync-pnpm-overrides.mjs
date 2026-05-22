import fs from "node:fs/promises";
import path from "node:path";

function parseTopLevelOverrides(lockfileText) {
  const lines = lockfileText.split(/\r?\n/);
  const overrides = {};
  let inOverrides = false;

  for (const line of lines) {
    if (!inOverrides) {
      if (line === "overrides:") {
        inOverrides = true;
      }
      continue;
    }

    if (line.trim() === "") {
      continue;
    }

    if (!line.startsWith("  ")) {
      break;
    }

    const trimmed = line.trim();
    const separatorIndex = trimmed.indexOf(": ");
    if (separatorIndex === -1) {
      throw new Error(`Malformed overrides entry in pnpm-lock.yaml: ${trimmed}`);
    }

    const rawKey = trimmed.slice(0, separatorIndex);
    const rawValue = trimmed.slice(separatorIndex + 2);
    overrides[unquoteYamlScalar(rawKey)] = unquoteYamlScalar(rawValue);
  }

  return Object.keys(overrides).length > 0 ? overrides : null;
}

function unquoteYamlScalar(value) {
  if (
    (value.startsWith("'") && value.endsWith("'")) ||
    (value.startsWith('"') && value.endsWith('"'))
  ) {
    return value.slice(1, -1);
  }
  return value;
}

function overridesEqual(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

async function main() {
  const targetDir = path.resolve(process.argv[2] ?? ".");
  const packageJsonPath = path.join(targetDir, "package.json");
  const lockfilePath = path.join(targetDir, "pnpm-lock.yaml");

  const [packageJsonText, lockfileText] = await Promise.all([
    fs.readFile(packageJsonPath, "utf8"),
    fs.readFile(lockfilePath, "utf8"),
  ]);

  const lockfileOverrides = parseTopLevelOverrides(lockfileText);
  if (!lockfileOverrides) {
    console.log("No top-level overrides found in pnpm-lock.yaml; nothing to sync.");
    return;
  }

  const packageJson = JSON.parse(packageJsonText);
  const currentOverrides = packageJson.pnpm?.overrides ?? null;

  if (overridesEqual(currentOverrides, lockfileOverrides)) {
    console.log("package.json pnpm.overrides already matches pnpm-lock.yaml.");
    return;
  }

  packageJson.pnpm = {
    ...(packageJson.pnpm ?? {}),
    overrides: lockfileOverrides,
  };

  await fs.writeFile(packageJsonPath, `${JSON.stringify(packageJson, null, 2)}\n`);
  console.log("Updated package.json pnpm.overrides from pnpm-lock.yaml.");
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
