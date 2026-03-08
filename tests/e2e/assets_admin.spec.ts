import { expect, test } from "@playwright/test";

import {
  addMembership,
  cleanupHarness,
  createAssetWithThumbnail,
  createAuthenticatedPage,
  createUser,
  setupHarness,
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
});
