import { test, expect } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import net from "node:net";

type CommandResult = {
  stdout: string;
  stderr: string;
};

function runCommand(
  command: string,
  args: string[],
  options: { env?: NodeJS.ProcessEnv; cwd?: string } = {},
): Promise<CommandResult> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      env: options.env ?? process.env,
      cwd: options.cwd,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (data) => {
      stdout += data.toString();
    });
    child.stderr.on("data", (data) => {
      stderr += data.toString();
    });

    child.on("error", (err) => {
      reject(err);
    });
    child.on("close", (code) => {
      if (code === 0) {
        resolve({ stdout, stderr });
      } else {
        reject(
          new Error(
            `command failed (${command} ${args.join(" ")}): ${stderr}\n${stdout}`,
          ),
        );
      }
    });
  });
}

async function loadEnvValue(name: string): Promise<string | undefined> {
  const value = process.env[name];
  if (value && value.length > 0) {
    return value;
  }

  try {
    const envrc = await readFile(".envrc", "utf8");
    const pattern = new RegExp(`export\\s+${name}=(?:\"([^\"]+)\"|([^\\s]+))`);
    const match = envrc.match(pattern);
    return match?.[1] ?? match?.[2];
  } catch {
    return undefined;
  }
}

async function reservePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close();
        reject(new Error("failed to reserve port"));
        return;
      }
      const port = address.port;
      server.close(() => resolve(port));
    });
  });
}

async function waitForPort(
  port: number,
  server: ReturnType<typeof spawn>,
  logs: { stdout: string; stderr: string },
  timeoutMs: number,
): Promise<void> {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    if (server.exitCode !== null) {
      throw new Error(
        `server exited early:\n${logs.stderr}\n${logs.stdout}`,
      );
    }

    try {
      await new Promise<void>((resolve, reject) => {
        const socket = net.connect(port, "127.0.0.1", () => {
          socket.end();
          resolve();
        });
        socket.once("error", reject);
      });
      return;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }

  throw new Error(
    `server did not become ready in time:\n${logs.stderr}\n${logs.stdout}`,
  );
}

type TestHarness = {
  tempRoot: string;
  dbPath: string;
  databaseUrl: string;
  env: NodeJS.ProcessEnv;
  tlsCertPath: string;
  tlsKeyPath: string;
  port: number;
  server: ReturnType<typeof spawn>;
  serverLogs: { stdout: string; stderr: string };
  siteId: string;
};

async function resolveTlsPaths(): Promise<{ tlsCertPath: string; tlsKeyPath: string }> {
  const tlsCertPath = await loadEnvValue("WEBSITES_TLS_CERT_PATH");
  const tlsKeyPath = await loadEnvValue("WEBSITES_TLS_KEY_PATH");
  if (!tlsCertPath || !tlsKeyPath) {
    throw new Error(
      "Missing WEBSITES_TLS_CERT_PATH/WEBSITES_TLS_KEY_PATH. Set env vars or update .envrc.",
    );
  }
  return { tlsCertPath, tlsKeyPath };
}

async function setupHarness(): Promise<TestHarness> {
  const tempRoot = await mkdtemp(path.join(tmpdir(), "websites-e2e-"));
  const dbPath = path.join(tempRoot, "database.sqlite");
  const databaseUrl = `sqlite://${dbPath}?mode=rwc`;
  const env = { ...process.env };
  const { tlsCertPath, tlsKeyPath } = await resolveTlsPaths();

  await runCommand(
    "cargo",
    [
      "run",
      "--",
      "--database-url",
      dbPath,
      "--tls-cert-path",
      tlsCertPath,
      "--tls-key-path",
      tlsKeyPath,
      "--frontend-url",
      "https://127.0.0.1",
      "init",
    ],
    { env },
  );
  const siteResult = await runCommand(
    "cargo",
    [
      "run",
      "--",
      "--database-url",
      dbPath,
      "--tls-cert-path",
      tlsCertPath,
      "--tls-key-path",
      tlsKeyPath,
      "--frontend-url",
      "https://127.0.0.1",
      "site",
      "create",
      "--short-name",
      "test",
      "--full-title",
      "Test Site",
      "--template-name",
      "default",
    ],
    { env },
  );

  const siteMatch = siteResult.stdout.match(/created site: ([^ ]+)/);
  if (!siteMatch) {
    throw new Error(`failed to parse site id: ${siteResult.stdout}`);
  }
  const siteId = siteMatch[1];

  const port = await reservePort();
  const serverLogs = { stdout: "", stderr: "" };
  const server = spawn(
    "cargo",
    [
      "run",
      "--",
      "--database-url",
      dbPath,
      "--tls-cert-path",
      tlsCertPath,
      "--tls-key-path",
      tlsKeyPath,
      "--frontend-url",
      `https://127.0.0.1:${port}`,
      "serve",
      "admin",
      "--listen",
      `127.0.0.1:${port}`,
    ],
    {
      env,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  server.stdout?.on("data", (data) => {
    serverLogs.stdout += data.toString();
  });
  server.stderr?.on("data", (data) => {
    serverLogs.stderr += data.toString();
  });
  await waitForPort(port, server, serverLogs, 30_000);

  return {
    tempRoot,
    dbPath,
    databaseUrl,
    env,
    tlsCertPath,
    tlsKeyPath,
    port,
    server,
    serverLogs,
    siteId,
  };
}

async function createUser(
  harness: TestHarness,
  subject: string,
): Promise<string> {
  const result = await runCommand(
    "cargo",
    [
      "run",
      "--",
      "--database-url",
      harness.dbPath,
      "--tls-cert-path",
      harness.tlsCertPath,
      "--tls-key-path",
      harness.tlsKeyPath,
      "--frontend-url",
      "https://127.0.0.1",
      "user",
      "create",
      "--subject",
      subject,
    ],
    { env: harness.env },
  );
  const match = result.stdout.match(/created user: ([^ ]+)/);
  if (!match) {
    throw new Error(`failed to parse user id: ${result.stdout}`);
  }
  return match[1];
}

async function addMembership(
  harness: TestHarness,
  userId: string,
  role: string,
): Promise<void> {
  await runCommand(
    "cargo",
    [
      "run",
      "--",
      "--database-url",
      harness.dbPath,
      "--tls-cert-path",
      harness.tlsCertPath,
      "--tls-key-path",
      harness.tlsKeyPath,
      "--frontend-url",
      "https://127.0.0.1",
      "site",
      "member-add",
      "--site-id",
      harness.siteId,
      "--user-id",
      userId,
      "--role",
      role,
    ],
    { env: harness.env },
  );
}

async function createTag(harness: TestHarness, name: string): Promise<void> {
  await runCommand(
    "cargo",
    [
      "run",
      "--",
      "--database-url",
      harness.dbPath,
      "--tls-cert-path",
      harness.tlsCertPath,
      "--tls-key-path",
      harness.tlsKeyPath,
      "--frontend-url",
      "https://127.0.0.1",
      "site",
      "tag-create",
      "--site-id",
      harness.siteId,
      "--name",
      name,
    ],
    { env: harness.env },
  );
}

async function seedSession(
  harness: TestHarness,
  subject: string,
): Promise<string> {
  const result = await runCommand(
    "cargo",
    [
      "run",
      "--bin",
      "session_seed",
      "--",
      "--database-url",
      harness.databaseUrl,
      "--user-sub",
      subject,
    ],
    { env: harness.env },
  );
  const sessionId = result.stdout.trim();
  if (!sessionId) {
    throw new Error("missing session id output");
  }
  return sessionId;
}

async function cleanupHarness(harness: TestHarness): Promise<void> {
  if (!harness.server.killed) {
    harness.server.kill("SIGTERM");
  }
  await rm(harness.tempRoot, { recursive: true, force: true });
}

test.describe("content new editor", () => {
  test.setTimeout(120_000);

  test("renders TipTap editor", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      const userId = await createUser(harness, "test-user");
      await addMembership(harness, userId, "owner");
      await createTag(harness, "news");
      const sessionId = await seedSession(harness, "test-user");

      const context = await browser.newContext({ ignoreHTTPSErrors: true });
      await context.addCookies([
        {
          name: "id",
          value: sessionId,
          url: `https://127.0.0.1:${harness.port}`,
        },
      ]);

      const page = await context.newPage();
      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
        { waitUntil: "domcontentloaded" },
      );

      await expect(page.locator("#editor")).toBeVisible();
      await page.locator(".ProseMirror").first().waitFor({ state: "visible" });
      await expect(page.locator("#page_content")).toBeHidden();
      await expect(page.locator("#tags")).toBeVisible();
      await expect(page.locator("#tags option", { hasText: "news" })).toBeVisible();
      await page.locator("#tags").selectOption("news");
      await expect(page.locator("#image_url")).toBeVisible();
      await expect(page.locator("#image_alt")).toBeVisible();
      await page.fill("#image_url", "https://example.com/test.png");
      await page.fill("#image_alt", "Test image");
      await page.getByRole("button", { name: "Image" }).click();
      await expect(
        page.locator('.ProseMirror img[src="https://example.com/test.png"]'),
      ).toBeVisible();
      await page.locator(".ProseMirror").click();
      await page.keyboard.type("Preview check");
      await expect(
        page.locator("[data-editor-preview]"),
      ).toBeHidden();
      await page.getByRole("button", { name: "Preview" }).click();
      await expect(
        page.locator("[data-editor-preview]"),
      ).toBeVisible();
      await expect(
        page.locator("[data-editor-preview-body]"),
      ).toContainText("Preview check");

      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/memberships`,
        { waitUntil: "domcontentloaded" },
      );
      await expect(page.getByText("Memberships")).toBeVisible();
      await expect(page.getByText("test-user")).toBeVisible();
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("allows access without membership", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      await createUser(harness, "intruder");
      const sessionId = await seedSession(harness, "intruder");

      const context = await browser.newContext({ ignoreHTTPSErrors: true });
      await context.addCookies([
        {
          name: "id",
          value: sessionId,
          url: `https://127.0.0.1:${harness.port}`,
        },
      ]);

      const page = await context.newPage();
      const response = await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
        { waitUntil: "domcontentloaded" },
      );

      expect(response).not.toBeNull();
      expect(response?.status()).toBe(200);
      await expect(page.locator("#editor")).toBeVisible();
    } finally {
      await cleanupHarness(harness);
    }
  });
});
