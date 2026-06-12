import { createServer } from "node:http";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";

import { expect, test } from "@playwright/test";

import {
	addMembership,
	cleanupHarness,
	captureResponsiveScreenshot,
	createAssetWithThumbnail,
	createAuthenticatedPage,
	createContent,
	createUser,
	expectElementInsetWithinContainer,
	expectNoHorizontalOverflow,
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

	test("missing image import page keeps section content padded", async ({
		browser,
	}, testInfo) => {
		const harness = await setupHarness();

		try {
			const subject = "missing-image-layout";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			const importRoot = path.join(harness.tempRoot, "mass-import-assets");
			const candidateDir = path.join(
				importRoot,
				"wp-content",
				"uploads",
				"2020",
			);
			await mkdir(candidateDir, { recursive: true });
			await writeFile(path.join(candidateDir, "hero.png"), tinyPngBytes);

			await createContent(harness, {
				pageType: "page",
				title: "Missing Image Layout",
				slug: "missing-image-layout",
				pageContent:
					'![Hero](https://example.com/wp-content/uploads/2020/hero.png)',
				creatorSub: subject,
			});

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`${harness.baseUrl}/admin/site/${harness.siteId}/settings`,
				{ waitUntil: "domcontentloaded" },
			);
			await page.getByLabel("Internal Domains").fill("example.com");
			await page.getByLabel("Mass Import Assets Path").fill(importRoot);
			await page.getByRole("button", { name: "Save settings" }).click();
			await expect(page).toHaveURL(
				`${harness.baseUrl}/admin/site/${harness.siteId}/settings`,
			);

			const missingPath = encodeURIComponent(
				"/wp-content/uploads/2020/hero.png",
			);
			const missingUrl = `${harness.baseUrl}/admin/site/${harness.siteId}/assets/mass-import/missing?path=${missingPath}`;
			await page.goto(missingUrl, { waitUntil: "domcontentloaded" });
			await expect(
				page.getByRole("heading", {
					level: 1,
					name: "Missing Image Import",
				}),
			).toBeVisible();

			const assertSubsectionPadding = async (minimumInset: number) => {
				const surfaceBox = await page
					.locator("main > section.surface")
					.boundingBox();
				const affectedBox = await page
					.getByRole("heading", { name: "Affected Content" })
					.boundingBox();
				const candidatesBox = await page
					.getByRole("heading", { name: "Local Candidates" })
					.boundingBox();
				expect(surfaceBox).not.toBeNull();
				expect(affectedBox).not.toBeNull();
				expect(candidatesBox).not.toBeNull();
				if (!surfaceBox || !affectedBox || !candidatesBox) {
					throw new Error("missing import layout boxes were not available");
				}
				expect(affectedBox.x - surfaceBox.x).toBeGreaterThanOrEqual(
					minimumInset,
				);
				expect(candidatesBox.x - surfaceBox.x).toBeGreaterThanOrEqual(
					minimumInset,
				);
			};

			await assertSubsectionPadding(23);
			await page.screenshot({
				path: testInfo.outputPath("missing-import-desktop.png"),
				fullPage: true,
			});

			await page.setViewportSize({ width: 430, height: 900 });
			await page.goto(missingUrl, { waitUntil: "domcontentloaded" });
			await assertSubsectionPadding(15);
			await page.screenshot({
				path: testInfo.outputPath("missing-import-mobile.png"),
				fullPage: true,
			});

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("mass import listing groups paths by first content page", async ({
		browser,
	}, testInfo) => {
		const harness = await setupHarness();

		try {
			const subject = "mass-import-listing";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			const importRoot = path.join(harness.tempRoot, "mass-import-assets");
			await mkdir(path.join(importRoot, "wp-content", "uploads", "2020"), {
				recursive: true,
			});
			await writeFile(
				path.join(importRoot, "wp-content", "uploads", "2020", "hero.png"),
				tinyPngBytes,
			);
			await writeFile(
				path.join(importRoot, "wp-content", "uploads", "2020", "other.png"),
				tinyPngBytes,
			);

			await createContent(harness, {
				pageType: "page",
				title: "Older Uses Hero",
				slug: "older-uses-hero",
				pageContent:
					'![Hero](https://example.com/wp-content/uploads/2020/hero.png)',
				creatorSub: subject,
			});
			await createContent(harness, {
				pageType: "page",
				title: "Newest Uses Grouped Assets",
				slug: "newest-uses-grouped-assets",
				pageContent:
					'![Other](/wp-content/uploads/2020/other.png) [Third](/wp-content/uploads/2020/third.gif)',
				creatorSub: subject,
			});

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`${harness.baseUrl}/admin/site/${harness.siteId}/settings`,
				{ waitUntil: "domcontentloaded" },
			);
			await page.getByLabel("Internal Domains").fill("example.com");
			await page.getByLabel("Mass Import Assets Path").fill(importRoot);
			await page.getByRole("button", { name: "Save settings" }).click();
			await expect(page).toHaveURL(
				`${harness.baseUrl}/admin/site/${harness.siteId}/settings`,
			);

			const listingUrl = `${harness.baseUrl}/admin/site/${harness.siteId}/assets/mass-import`;
			await page.goto(listingUrl, { waitUntil: "domcontentloaded" });
			await expect(
				page.getByRole("heading", { level: 1, name: "Mass Asset Import" }),
			).toBeVisible();
			await expect(
				page.getByText("First found on Newest Uses Grouped Assets"),
			).toBeVisible();
			await expect(
				page.getByText("First found on Older Uses Hero"),
			).toBeVisible();
			await expect(page.locator("body")).toContainText(
				"/wp-content/uploads/2020/other.png",
			);
			await expect(page.locator("body")).toContainText(
				"/wp-content/uploads/2020/third.gif",
			);
			await expect(page.locator("body")).toContainText(
				"/wp-content/uploads/2020/hero.png",
			);
			const assertTableContained = async () => {
				const surfaceBox = await page
					.locator("main > section.surface")
					.boundingBox();
				const tableScrollBox = await page
					.locator("[data-mass-import-table-scroll]")
					.boundingBox();
				expect(surfaceBox).not.toBeNull();
				expect(tableScrollBox).not.toBeNull();
				if (!surfaceBox || !tableScrollBox) {
					throw new Error("mass import table layout boxes were not available");
				}
				expect(tableScrollBox.x).toBeGreaterThanOrEqual(surfaceBox.x);
				expect(tableScrollBox.x + tableScrollBox.width).toBeLessThanOrEqual(
					surfaceBox.x + surfaceBox.width,
				);
			};
			await assertTableContained();
			await page.screenshot({
				path: testInfo.outputPath("mass-import-listing-desktop.png"),
				fullPage: true,
			});

			await page
				.getByRole("link", { name: "Newest Uses Grouped Assets" })
				.click();
			await expect(page).toHaveURL(
				new RegExp(
					`/admin/site/${harness.siteId}/assets/mass-import/content/[0-9a-f-]+$`,
				),
			);
			const contentImportUrl = page.url();
			await expect(
				page.getByRole("link", { name: "Mass asset import" }),
			).toHaveCount(1);
			await expect(page.getByRole("link", { name: "Open editor" })).toHaveCount(
				1,
			);
			await expect(
				page.getByRole("heading", {
					level: 2,
					name: "Missing Assets For Newest Uses Grouped Assets",
				}),
			).toBeVisible();
			await expect(page.locator("body")).toContainText(
				"/wp-content/uploads/2020/other.png",
			);
			await expect(page.locator("body")).toContainText(
				"/wp-content/uploads/2020/third.gif",
			);
			await expect(
				page.getByRole("button", { name: "Import selected" }),
			).toBeVisible();
			await expect(
				page.getByRole("button", { name: "Import this" }),
			).toBeVisible();

			const assertContentImportContained = async () => {
				const surfaceBox = await page
					.locator("main > section.surface")
					.boundingBox();
				const headingBox = await page
					.getByRole("heading", {
						level: 2,
						name: "Missing Assets For Newest Uses Grouped Assets",
					})
					.boundingBox();
				const pathRows = page.locator("[data-mass-import-content-path-row]");
				await expect(pathRows).toHaveCount(2);
				await expect(pathRows.first()).toHaveCSS("border-left-width", "0px");
				const candidateCard = page
					.locator("[data-mass-import-content-candidate]")
					.first();
				const candidateBox = await candidateCard.boundingBox();
				const previewWell = candidateCard.locator(
					"[data-mass-import-content-preview]",
				);
				const previewBox = await previewWell.boundingBox();
				expect(surfaceBox).not.toBeNull();
				expect(headingBox).not.toBeNull();
				expect(candidateBox).not.toBeNull();
				expect(previewBox).not.toBeNull();
				if (!surfaceBox || !headingBox || !candidateBox || !previewBox) {
					throw new Error("post mass import layout boxes were not available");
				}
				expect(headingBox.x - surfaceBox.x).toBeGreaterThanOrEqual(15);
				expect(candidateBox.x).toBeGreaterThanOrEqual(surfaceBox.x);
				expect(candidateBox.x + candidateBox.width).toBeLessThanOrEqual(
					surfaceBox.x + surfaceBox.width,
				);
				expect(previewBox.x).toBeGreaterThan(candidateBox.x);
				expect(previewBox.x + previewBox.width).toBeLessThan(
					candidateBox.x + candidateBox.width,
				);
				await expectElementInsetWithinContainer(
					candidateCard,
					previewWell,
					16,
				);
				await expectElementInsetWithinContainer(
					candidateCard,
					candidateCard.getByRole("button", { name: "Import this" }),
					16,
				);
			};
			await assertContentImportContained();
			await captureResponsiveScreenshot(
				page,
				testInfo,
				"mass-import-content-desktop",
			);

			await page.setViewportSize({ width: 430, height: 900 });
			await page.goto(listingUrl, { waitUntil: "domcontentloaded" });
			await expect(
				page.getByText("First found on Newest Uses Grouped Assets"),
			).toBeVisible();
			await assertTableContained();
			await expectNoHorizontalOverflow(page, 430);
			await page.screenshot({
				path: testInfo.outputPath("mass-import-listing-mobile.png"),
				fullPage: true,
			});

			await page.goto(contentImportUrl, { waitUntil: "domcontentloaded" });
			await expect(
				page.getByRole("heading", {
					level: 2,
					name: "Missing Assets For Newest Uses Grouped Assets",
				}),
			).toBeVisible();
			await assertContentImportContained();
			await expectNoHorizontalOverflow(page, 430);
			await captureResponsiveScreenshot(
				page,
				testInfo,
				"mass-import-content-mobile",
			);

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("uploads multiple assets from the admin upload page", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "asset-batch-uploader");
			await addMembership(harness, userId, "owner");

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"asset-batch-uploader",
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets/new`,
				{ waitUntil: "domcontentloaded" },
			);
			await page.locator('input[type="file"]').setInputFiles([
				{
					name: "batch-first.png",
					mimeType: "image/png",
					buffer: tinyPngBytes,
				},
				{
					name: "batch-second.png",
					mimeType: "image/png",
					buffer: tinyPngBytes,
				},
			]);
			await page.getByRole("button", { name: "Upload", exact: true }).click();

			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets`,
			);
			await expect(page.locator("body")).toContainText("batch-first.png");
			await expect(page.locator("body")).toContainText("batch-second.png");

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
