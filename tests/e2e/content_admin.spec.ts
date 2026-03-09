import { expect, test } from "@playwright/test";

import {
	addMembership,
	cleanupHarness,
	createAlias,
	createAuthenticatedApiContext,
	createAuthenticatedPage,
	createContent,
	createTag,
	createUser,
	setupHarness,
} from "./support";

test.describe("content admin", () => {
	test.setTimeout(120_000);

	test("shows a friendly 404 page for missing routes", async ({ browser }) => {
		const harness = await setupHarness();

		try {
			const context = await browser.newContext({ ignoreHTTPSErrors: true });
			const page = await context.newPage();
			const response = await page.goto(
				`https://127.0.0.1:${harness.port}/missing-page`,
				{ waitUntil: "domcontentloaded" },
			);

			expect(response).not.toBeNull();
			expect(response?.status()).toBe(404);
			await expect(
				page.getByRole("heading", { name: "Page Not Found" }),
			).toBeVisible();
			await expect(page.locator("body")).toContainText("could not be found");
			await expect(
				page.getByRole("link", { name: "Go back to the home page" }),
			).toHaveAttribute("href", "/");

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("shows content overview and metadata workflow pages", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "content-admin";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			const contentId = await createContent(harness, {
				pageType: "page",
				title: "Guide to Testing",
				slug: "guide-to-testing",
				pageContent: "First paragraph.\n\nSecond paragraph.",
				creatorSub: subject,
			});

			await createTag(harness, "guides");
			await createAlias(harness, {
				contentId,
				aliasPath: "/legacy/testing-guide",
			});

			const { api } = await createAuthenticatedApiContext(harness, subject);
			const updateResponse = await api.post(
				`/admin/site/${harness.siteId}/content/${contentId}/edit`,
				{
					form: {
						page_type: "page",
						title: "Guide to Testing",
						slug: "guide-to-testing",
						draft: "true",
						published_at: "",
						page_content: "First paragraph.\n\nSecond paragraph.",
						tag_list: "guides",
					},
				},
			);
			expect(updateResponse.status()).toBe(200);
			await api.dispose();

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(page).toHaveTitle("Content: /guide-to-testing - Test Site");
			await expect(
				page.getByRole("heading", { name: "Content: /guide-to-testing" }),
			).toBeVisible();
			await expect(page.locator(".page-site-indicator")).toHaveText("Test Site");
			await expect(page.locator("body")).toContainText("Primary Route");
			await expect(page.locator("body")).toContainText("/guide-to-testing");
			await expect(page.locator("body")).toContainText("/legacy/testing-guide");
			await expect(page.locator("body")).toContainText("guides");
			await expect(page.locator("body")).toContainText("latest is revision 2");

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/advanced`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}`,
			);
			await expect(
				page.getByRole("heading", { name: "Content: /guide-to-testing" }),
			).toBeVisible();
			await expect(page.getByRole("heading", { name: "Routes" })).toBeVisible();

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("does not show a site indicator on global admin pages", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "global-admin-page");
			await addMembership(harness, userId, "owner");

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"global-admin-page",
			);

			await page.goto(`https://127.0.0.1:${harness.port}/admin/sites`, {
				waitUntil: "domcontentloaded",
			});

			await expect(page).toHaveTitle("Sites");
			await expect(page.getByRole("heading", { name: "Sites" })).toBeVisible();
			await expect(page.locator(".page-site-indicator")).toHaveCount(0);

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});
});
