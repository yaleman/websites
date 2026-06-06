import { createServer } from "node:http";

import { expect, test, type Page } from "@playwright/test";

import {
	addMembership,
	cleanupHarness,
	createAlias,
	createAssetWithThumbnail,
	createAuthenticatedPage,
	createContent,
	createUser,
	setupHarness,
	tinyPngBytes,
	type TestHarness,
} from "./support";
import { defaultTimeout } from "./global_setup";

async function configureInternalDomains(
	page: Page,
	harness: TestHarness,
	domains: string,
): Promise<void> {
	await page.goto(
		`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings`,
		{ waitUntil: "domcontentloaded" },
	);
	await page.getByLabel("Internal Domains").fill(domains);
	await page.getByRole("button", { name: "Save settings" }).click();
	await expect(page).toHaveURL(
		`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings`,
	);
}

test.describe("content remediation", () => {
	test.setTimeout(defaultTimeout);

	test("scans content, rewrites internal links, and applies a selected asset", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "scan-editor";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			const linkedContentId = await createContent(harness, {
				pageType: "page",
				title: "Canonical Article",
				slug: "canonical-article",
				pageContent: "Target body",
				creatorSub: subject,
			});
			await createAlias(harness, {
				contentId: linkedContentId,
				aliasPath: "/legacy/article/",
			});

			const assetId = await createAssetWithThumbnail(harness, {
				originalFilename: "hero.png",
				storageBasename: "hero.png",
				thumbnailFilename: "hero_thumb.png",
			});

			const scanContentId = await createContent(harness, {
				pageType: "page",
				title: "Needs Cleanup",
				slug: "needs-cleanup",
				pageContent:
					'Visit https://example.com/legacy/article/ and <a href="https://example.com/legacy/article/">Legacy link</a><p>Paragraph</p><strong>Bold</strong><img src="https://example.com/uploads/legacy-hero.png" alt="Legacy hero" />',
				creatorSub: subject,
			});

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);
			await configureInternalDomains(page, harness, "example.com");

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/scan`,
				{ waitUntil: "domcontentloaded" },
			);
			await page.getByRole("button", { name: "Scan content" }).click();

			await expect(page.locator("body")).toContainText("Needs Cleanup");
			await expect(page.locator("body")).toContainText("Plain URL");
			await expect(page.locator("body")).toContainText("HTML link");
			await expect(page.locator("body")).toContainText("<p> tag");
			await expect(page.locator("body")).toContainText("<strong> tag");

			const imageIssue = page.locator("[data-remediation-issue]").first();
			await imageIssue.getByRole("button", { name: "Choose asset" }).click();
			const modal = page.getByRole("dialog", { name: "Select asset" });
			await expect(modal).toBeVisible();
			await modal.locator(".asset-card", { hasText: "hero.png" }).click();
			await modal.getByLabel("Variant").selectOption("thumbnail");
			await modal.getByRole("button", { name: "Use asset" }).click();
			await expect(modal).toBeHidden();

			await page.getByRole("button", { name: "Apply selected fixes" }).click();
			await expect(page.locator("body")).toContainText("Updated Content");
			await expect(page.locator("body")).toContainText("Needs Cleanup updated");

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${scanContentId}/edit`,
				{ waitUntil: "domcontentloaded" },
			);
			await page.getByRole("button", { name: "Markdown" }).click();
			const source = page.locator("#page_content");
			await expect(source).toContainText(
				"[Canonical Article](/legacy/article/)",
			);
			await expect(source).toContainText("[Legacy link](/legacy/article/)");
			await expect(source).toContainText("Paragraph");
			await expect(source).toContainText("**Bold**");
			await expect(source).toContainText(
				`[[asset id="${assetId}" variant="thumbnail" alt="Legacy hero"]]`,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${scanContentId}`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(page.locator("body")).toContainText("latest is revision 2");

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("offers remote image import when no asset match exists", async ({
		browser,
	}) => {
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
		const address = remoteServer.address();
		if (!address || typeof address === "string") {
			throw new Error("failed to start remote image server");
		}

		try {
			const subject = "scan-importer";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			const contentId = await createContent(harness, {
				pageType: "page",
				title: "Remote Image Page",
				slug: "remote-image-page",
				pageContent: `<strong>Leave Me Bold</strong><img src="http://127.0.0.1:${address.port}/remote-banner.png" alt="Remote banner" />`,
				creatorSub: subject,
			});

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/scan`,
				{ waitUntil: "domcontentloaded" },
			);
			await page.getByRole("button", { name: "Scan content" }).click();

			const remoteImportButton = page.getByRole("button", {
				name: "Import remote image",
			});
			await expect(remoteImportButton).toBeVisible();
			await remoteImportButton.click();

			await expect(page.locator("body")).toContainText(
				"Remote Image Page updated",
			);
			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(page.locator("body")).toContainText("remote-banner.png");

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				{ waitUntil: "domcontentloaded" },
			);
			await page.getByRole("button", { name: "Markdown" }).click();
			await expect(page.locator("#page_content")).toContainText("[[asset id=");
			await expect(page.locator("#page_content")).toContainText(
				'alt="Remote banner"',
			);
			await expect(page.locator("#page_content")).toContainText(
				"<strong>Leave Me Bold</strong>",
			);
			await expect(page.locator("#page_content")).not.toContainText(
				"**Leave Me Bold**",
			);

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

	test("limits results to the newest pages with issues", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "scan-limit-editor";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			await createContent(harness, {
				pageType: "page",
				title: "Oldest Broken Page",
				slug: "oldest-broken-page",
				pageContent: "Visit https://example.com/missing-old-page for more.",
				creatorSub: subject,
			});
			await createContent(harness, {
				pageType: "page",
				title: "Newest Broken Page",
				slug: "newest-broken-page",
				pageContent: "Visit https://example.com/missing-new-page for more.",
				creatorSub: subject,
			});

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);
			await configureInternalDomains(page, harness, "example.com");

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/scan`,
				{ waitUntil: "domcontentloaded" },
			);
			await page.getByLabel("Pages With Issues To Show").fill("1");
			await page.getByRole("button", { name: "Scan content" }).click();

			const body = page.locator("body");
			await expect(body).toContainText("Newest Broken Page");
			await expect(body).not.toContainText("Oldest Broken Page");
			await expect(body).toContainText("1");
			await expect(body).toContainText("issue pages shown");

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});
});
