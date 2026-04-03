import { createServer } from "node:http";
import { rm } from "node:fs/promises";
import path from "node:path";

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
import { defaultTimeout } from "./global_setup";

test.describe("assets admin", () => {
	test.setTimeout(defaultTimeout);

	test("shows asset metadata and the upload page asset browser", async ({
		browser,
	}) => {
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
			await expect(page).toHaveTitle("Assets - Test Site");
			await expect(page.getByRole("heading", { name: "Assets" })).toBeVisible();
			await expect(page.locator(".page-site-indicator")).toHaveText(
				"Test Site",
			);
			await expect(page.getByRole("img", { name: "banner.png" })).toBeVisible();
			await expect(page.locator("body")).toContainText("banner.png");
			await expect(page.locator("body")).toContainText("image/png");
			await expect(page.locator("body")).toContainText("800 x 600");
			await expect(page.locator("body")).toContainText("test-user");

			await page.setViewportSize({ width: 640, height: 960 });
			await expect(page.locator(".page-site-indicator")).toBeHidden();

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets/new`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(page).toHaveTitle("Upload Asset - Test Site");
			await expect(
				page.getByRole("button", { name: "Upload", exact: true }),
			).toBeVisible();
			await expect(
				page.getByRole("heading", { name: "Browse Assets" }),
			).toBeVisible();
			await expect(page.locator("body")).toContainText("banner.png");
			await expect(page.getByRole("img", { name: "banner.png" })).toBeVisible();

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("flags missing assets and supports in-place replacement", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "asset-replacer");
			await addMembership(harness, userId, "owner");
			const assetId = await createAssetWithThumbnail(harness, {
				originalFilename: "missing-banner.png",
				storageBasename: "missing-banner.png",
				thumbnailFilename: "missing-banner_thumb.png",
			});

			await rm(path.join(harness.uploadRoot, "missing-banner.png"), {
				force: true,
			});
			await rm(path.join(harness.uploadRoot, "missing-banner_thumb.png"), {
				force: true,
			});

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"asset-replacer",
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets`,
				{ waitUntil: "domcontentloaded" },
			);
			const assetRow = page.locator("tr", { hasText: "missing-banner.png" });
			await expect(
				assetRow.getByRole("link", { name: "missing" }),
			).toBeVisible();

			await page.getByRole("link", { name: "missing" }).click();
			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets/${assetId}/replace`,
			);
			await expect(
				page.getByRole("heading", { name: "Replace Asset" }),
			).toBeVisible();

			await page.locator('input[type="file"]').setInputFiles({
				name: "missing-banner.png",
				mimeType: "image/png",
				buffer: tinyPngBytes,
			});
			await page.getByRole("button", { name: "Replace", exact: true }).click();

			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets`,
			);
			await expect(
				assetRow.getByRole("img", { name: "missing-banner.png" }),
			).toBeVisible();
			await expect(
				assetRow.getByRole("link", { name: /^missing$/ }),
			).toHaveCount(0);

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
			await page
				.getByLabel("Import From URL")
				.fill(`http://127.0.0.1:${remoteAddress.port}/remote-banner.png`);
			await page.getByRole("button", { name: "Upload", exact: true }).click();

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
