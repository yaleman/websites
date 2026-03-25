import { expect, test } from "@playwright/test";
import { writeFile } from "node:fs/promises";
import path from "node:path";

import {
	addMembership,
	cleanupHarness,
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
});
