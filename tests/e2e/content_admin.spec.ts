import { expect, test } from "@playwright/test";

import {
	addMembership,
	cleanupHarness,
	createAlias,
	createAuthenticatedApiContext,
	createAuthenticatedPage,
	createContent,
	createSite,
	createTag,
	createUser,
	setupHarness,
} from "./support";
import { defaultTimeout } from "./global_setup";

test.describe("content admin", () => {
	test.setTimeout(defaultTimeout);

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
			await expect(page.locator(".page-site-indicator")).toHaveText(
				"Test Site",
			);
			await expect(page.locator("body")).toContainText("Primary Route");
			await expect(page.locator("body")).not.toContainText(
				"Scan this content for issues:",
			);
			await expect(page.getByRole("button", { name: "Scan for Issues" })).toHaveCount(0);
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

	test("filters and sorts the content overview list", async ({ browser }) => {
		const harness = await setupHarness();

		try {
			const subject = "content-list-filters";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			await createContent(harness, {
				pageType: "page",
				title: "Alpha Page",
				slug: "alpha-page",
				pageContent: "Alpha body",
				creatorSub: subject,
			});
			await createContent(harness, {
				pageType: "post",
				title: "Zulu Post",
				slug: "zulu-post",
				pageContent: "Zulu body",
				creatorSub: subject,
				draft: false,
			});
			await createContent(harness, {
				pageType: "page",
				title: "Beta Page",
				slug: "beta-page",
				pageContent: "Beta body",
				creatorSub: subject,
			});

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content`,
			);
			await expect(page.locator("tbody tr")).toHaveCount(3);

			await page.selectOption("#status", "draft");
			await page.getByRole("button", { name: "Filter" }).click();
			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content?page_type=all&status=draft&sort_by=created_desc`,
			);
			await expect(page.locator("tbody tr")).toHaveCount(2);
			await expect(page.locator("body")).not.toContainText("Zulu Post");

			await page.selectOption("#status", "published");
			await page.getByRole("button", { name: "Filter" }).click();
			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content?page_type=all&status=published&sort_by=created_desc`,
			);
			await expect(page.locator("tbody tr")).toHaveCount(1);
			await expect(page.locator("body")).toContainText("Zulu Post");
			await expect(page.locator("body")).not.toContainText("Alpha Page");
			await expect(page.locator("body")).not.toContainText("Beta Page");

			await page.selectOption("#page_type", "page");
			await page.selectOption("#status", "draft");
			await page.getByRole("button", { name: "Filter" }).click();
			await page
				.getByRole("columnheader", { name: "Title" })
				.getByRole("link")
				.click();
			await page
				.getByRole("columnheader", { name: "Title (asc)" })
				.getByRole("link")
				.click();

			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content?sort_by=title_desc&page_type=page&status=draft`,
			);

			await expect(
				page.getByRole("columnheader", { name: "Updated" }),
			).toBeVisible();
			const rows = page.locator("tbody tr");
			await expect(rows).toHaveCount(2);
			await expect(rows.nth(0)).toContainText("Beta Page");
			await expect(rows.nth(1)).toContainText("Alpha Page");
			await expect(page.locator("body")).not.toContainText("Zulu Post");

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("uses the nav search for site and global content search", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "nav-search";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			const otherSiteId = await createSite(harness, {
				shortName: "other-search-site",
				fullTitle: "Other Search Site",
			});

			await createContent(harness, {
				pageType: "page",
				title: "Current Site Match",
				slug: "current-site-match",
				pageContent: "cross-site-needle",
				creatorSub: subject,
			});
			await createContent(harness, {
				siteId: otherSiteId,
				pageType: "page",
				title: "Other Site Match",
				slug: "other-site-match",
				pageContent: "cross-site-needle",
				creatorSub: subject,
			});

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content`,
				{ waitUntil: "domcontentloaded" },
			);
			await page.locator("#nav-search-query").fill("cross-site-needle");
			await page.locator("#nav-search-query").press("Enter");

			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/search?q=cross-site-needle`,
			);
			await expect(page.locator("body")).toContainText("Current Site Match");
			await expect(page.locator("body")).not.toContainText("Other Site Match");
			await page.getByRole("link", { name: "Current Site Match" }).click();
			await expect(page).toHaveURL(
				new RegExp(
					`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/[^/]+/edit$`,
				),
			);

			await page.goto(`https://127.0.0.1:${harness.port}/admin`, {
				waitUntil: "domcontentloaded",
			});
			await page.locator("#nav-search-query").fill("cross-site-needle");
			await page.locator("#nav-search-query").press("Enter");

			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/search?q=cross-site-needle`,
			);
			await expect(page.locator("body")).toContainText("Current Site Match");
			await expect(page.locator("body")).toContainText("Other Site Match");
			await expect(page.locator("body")).toContainText("Test Site");
			await expect(page.locator("body")).toContainText("Other Search Site");
			await page.getByRole("link", { name: "Other Site Match" }).click();
			await expect(page).toHaveURL(
				new RegExp(
					`https://127.0.0.1:${harness.port}/admin/site/[^/]+/content/[^/]+/edit$`,
				),
			);

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

			await expect(page).toHaveURL(`https://127.0.0.1:${harness.port}/admin`);
			await expect(page).toHaveTitle("Admin Dashboard");
			await expect(
				page.getByRole("heading", { name: "Admin Dashboard" }),
			).toBeVisible();
			await expect(page.locator(".page-site-indicator")).toHaveCount(0);

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});
});
