import { expect, test } from "@playwright/test";
import { writeFile } from "node:fs/promises";
import path from "node:path";

import {
	addMembership,
	cleanupHarness,
	createAuthenticatedApiContext,
	createAuthenticatedPage,
	createUser,
	setupHarness,
} from "./support";
import { defaultTimeout } from "./global_setup";

test.describe("site settings", () => {
	test.setTimeout(defaultTimeout);

	test("shows delete site only to global admins and deletes the site", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const ownerSubject = "site-settings-owner";
			const ownerId = await createUser(harness, ownerSubject);
			await addMembership(harness, ownerId, "owner");

			const ownerSession = await createAuthenticatedPage(
				browser,
				harness,
				ownerSubject,
			);
			try {
			await ownerSession.page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(
				ownerSession.page.getByRole("link", { name: "Delete site" }),
			).toHaveCount(0);
			await expect(
				ownerSession.page.getByRole("link", { name: "Scan content" }),
			).toBeVisible();
			await expect(
				ownerSession.page.getByRole("link", {
					name: "Download site export JSON",
				}),
			).toHaveAttribute(
				"href",
				`/admin/site/${harness.siteId}/export.json`,
			);
			await ownerSession.page.getByRole("link", { name: "Scan content" }).click();
			await expect(ownerSession.page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/scan`,
			);
		} finally {
			await ownerSession.context.close();
		}

			const adminSession = await createAuthenticatedPage(
				browser,
				harness,
				`site-settings-admin-${Date.now()}`,
				true,
			);
			try {
				await adminSession.page.goto(
					`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings`,
					{ waitUntil: "domcontentloaded" },
				);
				await expect(
					adminSession.page.getByRole("link", { name: "Delete site" }),
				).toBeVisible();

				await adminSession.page
					.getByRole("link", { name: "Delete site" })
					.click();

				await expect(adminSession.page).toHaveURL(
					`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/delete`,
				);
				await expect(adminSession.page.locator("body")).toContainText(
					"Delete Test Site?",
				);

				await adminSession.page
					.getByRole("button", { name: "Confirm site deletion" })
					.click();

				await expect(adminSession.page).toHaveURL(
					`https://127.0.0.1:${harness.port}/admin?deleted=1`,
				);
				await expect(adminSession.page.locator("body")).toContainText(
					"Site deleted.",
				);
				await expect(adminSession.page.locator("body")).not.toContainText(
					"Test Site",
				);
			} finally {
				await adminSession.context.close();
			}
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("imports WordPress XML from site settings without duplicating content", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "wordpress-import-author";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "author");

			const xmlPath = path.join(harness.tempRoot, "wordpress-import.xml");
			await writeFile(
				xmlPath,
				`<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:wp="http://wordpress.org/export/1.2/" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <item>
      <title>Imported Post</title>
      <link>https://example.com/2020/01/imported-post/?p=123</link>
      <wp:post_id>123</wp:post_id>
      <wp:post_name>imported-post</wp:post_name>
      <wp:post_type>post</wp:post_type>
      <wp:status>publish</wp:status>
      <content:encoded><![CDATA[Hello world]]></content:encoded>
    </item>
  </channel>
</rss>
`,
			);

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(
				page.getByRole("link", { name: "WordPress import" }),
			).toHaveAttribute(
				"href",
				`/admin/site/${harness.siteId}/settings#wordpress-import`,
			);

			await page.getByRole("link", { name: "WordPress import" }).click();
			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings#wordpress-import`,
			);
			await expect(page.locator("#wordpress_xml")).toBeVisible();
			await page.setInputFiles("#wordpress_xml", xmlPath);
			await page.getByRole("button", { name: "Import WordPress XML" }).click();

			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings?imported=1`,
			);
			await expect(page.locator(".message--toast")).toContainText(
				"Imported 1 WordPress item.",
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(page.locator("tbody tr")).toHaveCount(1);
			await expect(page.locator("tbody")).toContainText("Imported Post");

			await page.getByRole("link", { name: "WordPress import" }).click();
			await page.setInputFiles("#wordpress_xml", xmlPath);
			await page.getByRole("button", { name: "Import WordPress XML" }).click();

			await expect(page).toHaveURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings?imported=0`,
			);
			await expect(page.locator(".message--toast")).toContainText(
				"No new WordPress items were imported.",
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(page.locator("tbody tr")).toHaveCount(1);

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("publish settings only show the active method and preserve values when switching", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const ownerSubject = "publish-settings-owner";
			const ownerId = await createUser(harness, ownerSubject);
			await addMembership(harness, ownerId, "owner");

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				ownerSubject,
			);

			try {
				await page.goto(
					`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/publish`,
					{ waitUntil: "domcontentloaded" },
				);

				const s3Panel = page.locator(
					'[data-publish-method-panel="s3_compatible"]',
				);
				const rsyncPanel = page.locator('[data-publish-method-panel="rsync_ssh"]');

				await page.selectOption("#method", "s3_compatible");
				await expect(s3Panel).toBeVisible();
				await expect(rsyncPanel).toBeHidden();

				await page.getByLabel("Bucket").fill("example-bucket");
				await page.getByLabel("Prefix").fill("site");
				await page.getByLabel("Region").fill("us-east-1");
				await page.getByLabel("Access Key ID").fill("access-key");
				await page.getByLabel("Secret Access Key").fill("secret-key");
				await page.getByLabel("Force path-style requests").check();

				await page.selectOption("#method", "rsync_ssh");
				await expect(s3Panel).toBeHidden();
				await expect(rsyncPanel).toBeVisible();

				await page.getByLabel("SSH Host").fill("publish.example.com");
				await page.getByLabel("SSH User").fill("deploy");
				await page.getByLabel("SSH Port").fill("2222");
				await page.getByLabel("Remote Path").fill("/var/www/example");
				await page.getByLabel("Identity File").fill("/tmp/id_ed25519");

				await page.selectOption("#method", "s3_compatible");
				await expect(s3Panel).toBeVisible();
				await expect(rsyncPanel).toBeHidden();
				await expect(page.getByLabel("Bucket")).toHaveValue("example-bucket");
				await expect(page.getByLabel("Prefix")).toHaveValue("site");
				await expect(page.getByLabel("Region")).toHaveValue("us-east-1");
				await expect(page.getByLabel("Access Key ID")).toHaveValue("access-key");
				await expect(page.getByLabel("Secret Access Key")).toHaveValue("secret-key");
				await expect(
					page.getByLabel("Force path-style requests"),
				).toBeChecked();

				await page.selectOption("#method", "disabled");
				await expect(s3Panel).toBeHidden();
				await expect(rsyncPanel).toBeHidden();

				await page.selectOption("#method", "s3_compatible");
				await expect(s3Panel).toBeVisible();
				await expect(rsyncPanel).toBeHidden();
				await expect(page.getByLabel("Bucket")).toHaveValue("example-bucket");
				await expect(page.locator('input[name="ssh_host"]')).toHaveValue(
					"publish.example.com",
				);
			} finally {
				await context.close();
			}
			} finally {
				await cleanupHarness(harness);
			}
		});

	test("publish on render hides the navbar publish button when enabled", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "publish-on-render-owner";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			try {
				await page.goto(
					`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings`,
					{ waitUntil: "domcontentloaded" },
				);

				const publishButton = page.getByRole("link", { name: "Publish Site" });
				await expect(publishButton).toBeVisible();
				await expect(page.getByLabel("Publish on render")).not.toBeChecked();

				await page.getByLabel("Publish on render").check();
				await page.getByRole("button", { name: "Save settings" }).click();

				await page.goto(
					`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content`,
					{ waitUntil: "domcontentloaded" },
				);
				await expect(
					page.getByRole("link", { name: "Publish Site" }),
				).toHaveCount(0);

				await page.goto(
					`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings`,
					{ waitUntil: "domcontentloaded" },
				);
				await expect(page.getByLabel("Publish on render")).toBeChecked();
				await page.getByLabel("Publish on render").uncheck();
				await page.getByRole("button", { name: "Save settings" }).click();

				await page.goto(
					`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content`,
					{ waitUntil: "domcontentloaded" },
				);
				await expect(publishButton).toBeVisible();
			} finally {
				await context.close();
			}
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("prompts to replace an existing site export before importing", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "site-import-admin";
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
				true,
			);
			const apiContext = await createAuthenticatedApiContext(
				harness,
				subject,
				true,
			);

			try {
				await page.goto(
					`https://127.0.0.1:${harness.port}/admin/sites/import`,
					{ waitUntil: "domcontentloaded" },
				);

				const exportResponse = await apiContext.api.get(
					`/admin/site/${harness.siteId}/export.json`,
				);
				expect(exportResponse.ok()).toBe(true);
				const exportJson = await exportResponse.text();

				const jsonPath = path.join(harness.tempRoot, "site-export.json");
				await writeFile(jsonPath, exportJson);

				await page.goto(
					`https://127.0.0.1:${harness.port}/admin/sites/import`,
					{ waitUntil: "domcontentloaded" },
				);
				await page.setInputFiles("#file", jsonPath);

				const prompt = page.locator("[data-site-import-prompt]");
				await expect(prompt).toBeVisible();
				await expect(page.locator("[data-site-import-details]")).toContainText(
					"already exists",
				);
				const submit = page.locator("[data-site-import-submit]");
				await expect(submit).toBeDisabled();

				await page.getByLabel("Replace existing site").check();
				await expect(submit).toBeEnabled();

				await submit.click();
				await expect(page).toHaveURL(
					`https://127.0.0.1:${harness.port}/admin?imported=1`,
				);
				await expect(page.locator("body")).toContainText(
					"Site import complete.",
				);
			} finally {
				await apiContext.api.dispose();
				await context.close();
			}
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("falls back to the dashboard when the import lookup route is unavailable", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "site-import-fallback";
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
				true,
			);
			const apiContext = await createAuthenticatedApiContext(
				harness,
				subject,
				true,
			);

			try {
				await page.route("**/admin/sites/import/check**", async (route) => {
					await route.fulfill({
						status: 404,
						contentType: "text/plain",
						body: "Not Found",
					});
				});

				const exportResponse = await apiContext.api.get(
					`/admin/site/${harness.siteId}/export.json`,
				);
				expect(exportResponse.ok()).toBe(true);
				const exportJson = await exportResponse.text();

				const jsonPath = path.join(harness.tempRoot, "site-export-fallback.json");
				await writeFile(jsonPath, exportJson);

				await page.goto(
					`https://127.0.0.1:${harness.port}/admin/sites/import`,
					{ waitUntil: "domcontentloaded" },
				);
				await page.setInputFiles("#file", jsonPath);

				await expect(page.locator("[data-site-import-prompt]")).toBeVisible();
				await expect(page.locator("[data-site-import-details]")).toContainText(
					"already exists",
				);
				await expect(page.locator("[data-site-import-status]")).toBeHidden();
			} finally {
				await apiContext.api.dispose();
				await context.close();
			}
		} finally {
			await cleanupHarness(harness);
		}
	});
});
