import { expect, test } from "@playwright/test";

import {
	addMembership,
	cleanupHarness,
	createAuthenticatedPage,
	createUser,
	setupHarness,
} from "./support";

test.describe("site settings", () => {
	test.setTimeout(120_000);

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

				await adminSession.page.getByRole("link", { name: "Delete site" }).click();

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
});
