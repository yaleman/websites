import { test, expect } from "@playwright/test";
import { mkdtemp, rm } from "node:fs/promises";
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

function waitForOutput(
  child: ReturnType<typeof spawn>,
  pattern: RegExp,
  timeoutMs: number,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error("server did not become ready in time"));
    }, timeoutMs);

    const onData = (data: Buffer) => {
      const text = data.toString();
      if (pattern.test(text)) {
        clearTimeout(timeout);
        cleanup();
        resolve();
      }
    };

    const cleanup = () => {
      child.stdout?.off("data", onData);
      child.stderr?.off("data", onData);
    };

    child.stdout?.on("data", onData);
    child.stderr?.on("data", onData);
    child.on("exit", () => {
      clearTimeout(timeout);
      cleanup();
    });
  });
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

test.describe("content new editor", () => {
  test.setTimeout(120_000);

  test("renders Milkdown editor", async ({ browser }) => {
    const tempRoot = await mkdtemp(path.join(tmpdir(), "websites-e2e-"));
    const dbPath = path.join(tempRoot, "database.sqlite");
    const databaseUrl = `sqlite://${dbPath}?mode=rwc`;
    const env = { ...process.env };

    let server: ReturnType<typeof spawn> | null = null;

    try {
      await runCommand(
        "cargo",
        ["run", "--", "init", "--database-url", dbPath],
        { env },
      );
      const siteResult = await runCommand(
        "cargo",
        [
          "run",
          "--",
          "site",
          "create",
          "--database-url",
          dbPath,
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

      const sessionResult = await runCommand(
        "cargo",
        [
          "run",
          "--bin",
          "session_seed",
          "--",
          "--database-url",
          databaseUrl,
          "--user-sub",
          "test-user",
        ],
        { env },
      );
      const sessionId = sessionResult.stdout.trim();
      if (!sessionId) {
        throw new Error("missing session id output");
      }

      const port = await reservePort();
      server = spawn(
        "cargo",
        [
          "run",
          "--",
          "serve",
          "admin",
          "--database-url",
          dbPath,
          "--listen",
          `127.0.0.1:${port}`,
        ],
        {
          env,
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      await waitForOutput(server, /admin server listening on http:\/\//, 30_000);

      const context = await browser.newContext();
      await context.addCookies([
        {
          name: "id",
          value: sessionId,
          url: `http://127.0.0.1:${port}`,
          path: "/",
        },
      ]);

      const page = await context.newPage();
      await page.goto(
        `http://127.0.0.1:${port}/admin/site/${siteId}/content/new`,
        { waitUntil: "domcontentloaded" },
      );

      await expect(page.locator("#editor")).toBeVisible();
      await page.locator(".milkdown .editor").first().waitFor({ state: "visible" });
      await expect(page.locator("#page_content")).toBeHidden();
    } finally {
      if (server && !server.killed) {
        server.kill("SIGTERM");
      }
      await rm(tempRoot, { recursive: true, force: true });
    }
  });
});
