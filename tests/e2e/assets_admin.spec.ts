import { createServer } from "node:http";

import { expect, test } from "@playwright/test";

import {
  addMembership,
  cleanupHarness,
  createAssetWithThumbnail,
  createAuthenticatedPage,
  createUser,
  setupHarness,
  tinyPngBytes,
} from "./support";

test.describe("assets admin", () => {
  test.setTimeout(120_000);

  test("shows asset metadata and recent uploads", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      const userId = await createUser(harness, "asset-admin");
      await addMembership(harness, userId, "owner");
      await createAssetWithThumbnail(harness, {
        originalFilename: "banner.png",
        storageBasename: "banner.png",
        thumbnailFilename: "banner_thumb.png",
      });

      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "asset-admin",
      );

      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets`,
        { waitUntil: "domcontentloaded" },
      );
      await expect(page.getByRole("heading", { name: "Site Assets (test)" })).toBeVisible();
      await expect(page.getByRole("img", { name: "banner.png" })).toBeVisible();
      await expect(page.locator("body")).toContainText("banner.png");
      await expect(page.locator("body")).toContainText("image/png");
      await expect(page.locator("body")).toContainText("800 x 600");
      await expect(page.locator("body")).toContainText("test-user");

      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets/new`,
        { waitUntil: "domcontentloaded" },
      );
      await expect(page.getByRole("button", { name: "Upload asset" })).toBeVisible();
      await expect(page.getByRole("heading", { name: "Recent Uploads" })).toBeVisible();
      await expect(page.locator("body")).toContainText("banner.png");
      await expect(page.getByRole("img", { name: "banner.png" })).toBeVisible();

      await context.close();
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("imports an asset from a remote url", async ({ browser }) => {
    const harness = await setupHarness();
    const remoteServer = createServer((request, response) => {
      if (request.url === "/remote-banner.png") {
        response.writeHead(200, {
          "Content-Type": "image/png",
          "Content-Length": tinyPngBytes.length,
        });
        response.end(tinyPngBytes);
        return;
      }

      response.writeHead(404);
      response.end("not found");
    });
    await new Promise<void>((resolve) => {
      remoteServer.listen(0, "127.0.0.1", () => resolve());
    });
    const remoteAddress = remoteServer.address();
    if (!remoteAddress || typeof remoteAddress === "string") {
      throw new Error("failed to start remote asset server");
    }

    try {
      const userId = await createUser(harness, "asset-importer");
      await addMembership(harness, userId, "owner");

      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "asset-importer",
      );

      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets/new`,
        { waitUntil: "domcontentloaded" },
      );
      await page.getByLabel("Import From URL").fill(
        `http://127.0.0.1:${remoteAddress.port}/remote-banner.png`,
      );
      await page.getByRole("button", { name: "Upload asset" }).click();

      await expect(page).toHaveURL(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets`,
      );
      await expect(page.locator("body")).toContainText("remote-banner.png");
      await expect(page.locator("body")).toContainText("image/png");
      await expect(page.locator("body")).toContainText("asset-importer");

      await context.close();
    } finally {
      await cleanupHarness(harness);
      await new Promise<void>((resolve, reject) => {
        remoteServer.close((error) => {
          if (error) {
            reject(error);
            return;
          }
          resolve();
        });
      });
    }
  });
});
