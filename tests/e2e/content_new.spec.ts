import { expect, test } from "@playwright/test";

import {
  addMembership,
  cleanupHarness,
  createAssetWithThumbnail,
  createAuthenticatedPage,
  createContent,
  createTag,
  createUser,
  setupHarness,
} from "./support";

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
      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "test-user",
      );
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
      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "intruder",
      );
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
      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "site-admin",
        true,
      );
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
      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "editor-user",
      );
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
      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "tag-editor",
      );
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
      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "tag-admin",
      );
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
      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "profile-user",
      );
      const response = await page.goto(
        `https://127.0.0.1:${harness.port}/admin/users/${userId}`,
        { waitUntil: "domcontentloaded" },
      );

      expect(response).not.toBeNull();
      expect(response?.status()).toBe(200);
      await expect(
        page.getByRole("heading", { name: "User Profile: profile-user" }),
      ).toBeVisible();
      await expect.poll(() => page.content()).toContain(`Database ID: ${userId}`);
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
      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "global-admin",
        true,
      );
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
      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        "viewer-user",
      );
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
