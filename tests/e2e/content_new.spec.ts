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

const oidcTestArgs = [
  "--client-id",
  "playwright-test-client",
  "--discovery-url",
  "https://example.com/.well-known/openid-configuration",
] as const;

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
      ...oidcTestArgs,
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
      ...oidcTestArgs,
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
      ...oidcTestArgs,
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
      ...oidcTestArgs,
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
      ...oidcTestArgs,
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
      ...oidcTestArgs,
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

async function createAssetWithThumbnail(
  harness: TestHarness,
  {
    originalFilename,
    storageBasename,
    thumbnailFilename,
  }: {
    originalFilename: string;
    storageBasename: string;
    thumbnailFilename: string;
  },
): Promise<void> {
  const createResult = await runCommand(
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
      ...oidcTestArgs,
      "asset",
      "create",
      "--site-id",
      harness.siteId,
      "--uploader-sub",
      "test-user",
      "--original-filename",
      originalFilename,
      "--storage-basename",
      storageBasename,
      "--mime-type",
      "image/png",
      "--byte-length",
      "128",
      "--width",
      "800",
      "--height",
      "600",
    ],
    { env: harness.env },
  );
  const match = createResult.stdout.match(/created asset: ([^ ]+)/);
  if (!match) {
    throw new Error(`failed to parse asset id: ${createResult.stdout}`);
  }
  const assetId = match[1];

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
      ...oidcTestArgs,
      "asset",
      "variant-create",
      "--asset-id",
      assetId,
      "--variant-kind",
      "thumbnail",
      "--filename",
      thumbnailFilename,
      "--mime-type",
      "image/png",
      "--byte-length",
      "64",
      "--width",
      "320",
      "--height",
      "240",
    ],
    { env: harness.env },
  );
}

async function createContent(
  harness: TestHarness,
  {
    pageType,
    title,
    slug,
    pageContent,
    creatorSub,
    draft = true,
  }: {
    pageType: string;
    title: string;
    slug: string;
    pageContent: string;
    creatorSub: string;
    draft?: boolean;
  },
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
      ...oidcTestArgs,
      "content",
      "create",
      "--site-id",
      harness.siteId,
      "--page-type",
      pageType,
      "--title",
      title,
      "--slug",
      slug,
      "--page-content",
      pageContent,
      "--creator-sub",
      creatorSub,
      ...(draft ? ["--draft"] : []),
    ],
    { env: harness.env },
  );
  const match = result.stdout.match(/created content: ([^ ]+)/);
  if (!match) {
    throw new Error(`failed to parse content id: ${result.stdout}`);
  }
  return match[1];
}

async function seedSession(
  harness: TestHarness,
  subject: string,
  setAdmin = false,
): Promise<string> {
  const args = [
    "run",
    "--bin",
    "session_seed",
    "--",
    "--database-url",
    harness.databaseUrl,
    "--user-sub",
    subject,
  ];
  if (setAdmin) {
    args.push("--set-admin");
  }
  const result = await runCommand(
    "cargo",
    args,
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
      const asset = {
        originalFilename: "test-image.png",
        storageBasename: "test-image.png",
        thumbnailFilename: "test-image_thumb.png",
      };
      await createAssetWithThumbnail(harness, asset);
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
      await expect(page.locator('#tag-suggestions option[value="news"]')).toBeAttached();
      await page.locator("#tags").fill("news");
      await page.locator("#tags").press("Enter");
      await expect(page.locator('[data-tag-chip="news"]')).toBeVisible();
      await page.getByRole("button", { name: "Image" }).click();
      const modal = page.getByRole("dialog", { name: "Insert image" });
      await expect(modal).toBeVisible();
      const assetCard = modal.locator(".asset-card", {
        hasText: asset.originalFilename,
      });
      await expect(assetCard).toBeVisible();
      await assetCard.click();
      await modal.getByLabel("Alt text").fill("Test image");
      await modal.getByRole("button", { name: "Insert image" }).click();
      await expect(modal).toBeHidden();
      await expect(
        page.locator(
          `.ProseMirror a[href="/media/images/${asset.storageBasename}"] img[src="/media/images/${asset.thumbnailFilename}"]`,
        ),
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
      await expect(page.locator("[data-editor-source-panel]")).toBeHidden();
      await page.getByRole("button", { name: "Source" }).click();
      await expect(page.locator("[data-editor-source-panel]")).toBeVisible();
      await expect(page.locator("#page_content")).toBeVisible();
      await page.locator("#page_content").fill("## Raw heading\n\nraw body");
      await expect(page.locator("[data-editor-preview-body]")).toContainText(
        "Raw heading",
      );
      await expect(page.locator("[data-editor-preview-body]")).toContainText(
        "raw body",
      );
      await expect(page.locator(".ProseMirror")).toContainText("Raw heading");
      await expect(page.locator(".ProseMirror")).toContainText("raw body");

      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/memberships`,
        { waitUntil: "domcontentloaded" },
      );
      await expect(
        page.getByRole("heading", { name: "Memberships", exact: true }),
      ).toBeVisible();
      await expect(page.locator('[aria-label="test-user"]')).toBeVisible();
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
      expect(response?.status()).toBe(401);
      await expect(page.locator("body")).toContainText(
        `missing membership for site ${harness.siteId}`,
      );
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("allows system admin access without membership", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      const sessionId = await seedSession(harness, "site-admin", true);

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

  test("saves back to the source editor and shows a toast", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      const userId = await createUser(harness, "editor-user");
      await addMembership(harness, userId, "owner");
      const contentId = await createContent(harness, {
        pageType: "page",
        title: "Existing page",
        slug: "existing-page",
        pageContent: "Initial body",
        creatorSub: "editor-user",
      });
      const sessionId = await seedSession(harness, "editor-user");

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
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/source`,
        { waitUntil: "domcontentloaded" },
      );

      await page.getByRole("button", { name: "Source" }).click();
      await page.locator("#page_content").fill("Updated body from source mode");
      await page.getByRole("button", { name: "Save content" }).click();

      await expect(page).toHaveURL(
        new RegExp(`/admin/site/${harness.siteId}/content/${contentId}/source`),
      );
      const toast = page.locator(".message--toast");
      await expect(toast).toBeVisible();
      await expect(toast).toContainText("Content saved.");
      await expect(page.locator("#page_content")).toHaveValue(
        "Updated body from source mode",
      );
      await expect(toast).toBeHidden({ timeout: 4000 });
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("updates tags from the source editor", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      const userId = await createUser(harness, "tag-editor");
      await addMembership(harness, userId, "owner");
      await createTag(harness, "docs");
      await createTag(harness, "news");
      const contentId = await createContent(harness, {
        pageType: "page",
        title: "Tagged page",
        slug: "tagged-page",
        pageContent: "Initial body",
        creatorSub: "tag-editor",
      });
      const sessionId = await seedSession(harness, "tag-editor");

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
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/source`,
        { waitUntil: "domcontentloaded" },
      );

      await page.locator("#tags").fill("docs");
      await page.locator("#tags").press("Enter");
      await page.locator("#tags").fill("news");
      await page.locator("#tags").press("Enter");
      await Promise.all([
        page.waitForResponse(
          (response) =>
            response.request().method() === "POST" &&
            response.url() ===
              `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/source`,
        ),
        page.getByRole("button", { name: "Save content" }).click(),
      ]);
      await page.waitForLoadState("domcontentloaded");
      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/source`,
        { waitUntil: "domcontentloaded" },
      );

      await expect(page.locator('[data-tag-chip="docs"]')).toBeVisible();
      await expect(page.locator('[data-tag-chip="news"]')).toBeVisible();

      await page.locator("#tags").fill("guides");
      await page.locator("#tags").press("Enter");
      await page.locator('[data-tag-chip="docs"]').click();
      await page.locator('[data-tag-chip="news"]').click();
      await Promise.all([
        page.waitForResponse(
          (response) =>
            response.request().method() === "POST" &&
            response.url() ===
              `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/source`,
        ),
        page.getByRole("button", { name: "Save content" }).click(),
      ]);
      await page.waitForLoadState("domcontentloaded");
      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/source`,
        { waitUntil: "domcontentloaded" },
      );
      await expect(page.locator('[data-tag-chip="guides"]')).toBeVisible();
      await expect(page.locator('[data-tag-chip="docs"]')).toHaveCount(0);
      await expect(page.locator('[data-tag-chip="news"]')).toHaveCount(0);
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("creates and deletes tags from the tags admin page", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      const userId = await createUser(harness, "tag-admin");
      await addMembership(harness, userId, "owner");
      const sessionId = await seedSession(harness, "tag-admin");

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
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/tags`,
        { waitUntil: "domcontentloaded" },
      );

      await page.getByLabel("New tag").fill("release-notes");
      await page.getByRole("button", { name: "Create tag" }).click();
      await expect(page.getByRole("cell", { name: "release-notes" })).toBeVisible();

      await page
        .locator("tr", { has: page.getByRole("cell", { name: "release-notes" }) })
        .getByRole("button", { name: "Delete" })
        .click();
      await expect(page.getByRole("cell", { name: "release-notes" })).toHaveCount(0);
    } finally {
      await cleanupHarness(harness);
    }
  });
});

test.describe("user profile", () => {
  test.setTimeout(120_000);

  test("lets a user view their own profile details and memberships", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      const userId = await createUser(harness, "profile-user");
      await addMembership(harness, userId, "author");
      const sessionId = await seedSession(harness, "profile-user");

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
        `https://127.0.0.1:${harness.port}/admin/users/${userId}`,
        { waitUntil: "domcontentloaded" },
      );

      expect(response).not.toBeNull();
      expect(response?.status()).toBe(200);
      await expect(
        page.getByRole("heading", { name: "User Profile: profile-user" }),
      ).toBeVisible();
      await expect(page.getByRole("cell", { name: userId })).toBeVisible();
      await expect(page.getByRole("cell", { name: "profile-user" })).toBeVisible();
      await expect(page.getByRole("cell", { name: "No" })).toBeVisible();
      await expect(page.getByRole("link", { name: "Test Site" })).toBeVisible();
      await expect(page.getByRole("cell", { name: "Author" })).toBeVisible();
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("lets a system admin view another user's profile", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      const targetUserId = await createUser(harness, "target-user");
      await addMembership(harness, targetUserId, "viewer");
      const sessionId = await seedSession(harness, "global-admin", true);

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
        `https://127.0.0.1:${harness.port}/admin/users/${targetUserId}`,
        { waitUntil: "domcontentloaded" },
      );

      expect(response).not.toBeNull();
      expect(response?.status()).toBe(200);
      await expect(
        page.getByRole("heading", { name: "User Profile: target-user" }),
      ).toBeVisible();
      await expect(page.getByRole("cell", { name: "target-user" })).toBeVisible();
      await expect(page.getByRole("cell", { name: "Viewer" })).toBeVisible();
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("blocks a non-admin user from viewing another user's profile", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      await createUser(harness, "viewer-user");
      const targetUserId = await createUser(harness, "private-user");
      const sessionId = await seedSession(harness, "viewer-user");

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
        `https://127.0.0.1:${harness.port}/admin/users/${targetUserId}`,
        { waitUntil: "domcontentloaded" },
      );

      expect(response).not.toBeNull();
      expect(response?.status()).toBe(401);
      await expect(page.locator("body")).toContainText(
        "cannot view another user's profile",
      );
    } finally {
      await cleanupHarness(harness);
    }
  });
});
