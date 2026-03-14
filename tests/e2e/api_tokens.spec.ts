import { expect, request as playwrightRequest, test } from "@playwright/test";

import {
	cleanupHarness,
	createAuthenticatedPage,
	createMembership,
	createUser,
	setupHarness,
} from "./support";
import { defaultTimeout } from "./global_setup";

test.describe("api token admin flows", () => {
	test.setTimeout(defaultTimeout);

	test("global admin can manually create a user from the UI", async ({ browser }) => {
		const harness = await setupHarness();
		try {
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"ui-admin",
				true,
			);

			await page.goto(`${harness.baseUrl}/admin/users`);
			await page.getByLabel("Subject").fill("manual-ui-user");
			await page.getByLabel("Email").fill("manual-ui-user@example.com");
			await page.getByLabel("Display Name").fill("Manual UI User");
			await page.getByLabel("Global admin").check();
			await page.getByRole("button", { name: "Create user" }).click();

			await expect(
				page.getByRole("link", { name: "manual-ui-user" }),
			).toBeVisible();
			await expect(page.getByText("Manual UI User")).toBeVisible();
			await expect(page.getByText("manual-ui-user@example.com")).toBeVisible();

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("issued bearer token tracks use, respects live permissions, and can be revoked", async ({
		browser,
	}) => {
		const harness = await setupHarness();
		try {
			const targetSubject = "api-token-user";
			const targetUserId = await createUser(harness, targetSubject);
			const membershipId = await createMembership(harness, targetUserId, "author");

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"token-admin",
				true,
			);

			await page.goto(`${harness.baseUrl}/admin/users/${targetUserId}`);
			await page.getByLabel("Label").fill("playwright token");
			await page.getByLabel("Current user access").check();
			await page.getByRole("button", { name: "Issue token" }).click();

			const token = await page.getByLabel("Issued Token").inputValue();
			expect(token.length).toBeGreaterThan(20);

			const bearerApi = await playwrightRequest.newContext({
				baseURL: harness.baseUrl,
				ignoreHTTPSErrors: true,
				extraHTTPHeaders: {
					Authorization: `Bearer ${token}`,
				},
			});

			const success = await bearerApi.get(
				`/api/site/${harness.siteId}/assets/library`,
			);
			expect(success.status()).toBe(200);

			await page.goto(`${harness.baseUrl}/admin/users/${targetUserId}`);
			const tokenRow = page
				.getByRole("row", { name: /playwright token/ })
				.first();
			await expect(tokenRow).toBeVisible();
			await expect(tokenRow).toContainText("Current user access");
			await expect(tokenRow.locator("td").nth(3)).not.toHaveText("n/a");

			const updateResponse = await page.request.post(
				`${harness.baseUrl}/admin/site/${harness.siteId}/memberships/${membershipId}/update`,
				{
					form: {
						role: "viewer",
					},
				},
			);
			expect(updateResponse.status()).toBe(200);

			const downgraded = await bearerApi.get(
				`/api/site/${harness.siteId}/assets/library`,
			);
			expect(downgraded.status()).toBe(403);
			expect(downgraded.headers()["www-authenticate"]).toContain(
				"insufficient_scope",
			);

			await tokenRow.getByRole("button", { name: "Revoke" }).click();
			await expect(page).toHaveURL(
				new RegExp(`/admin/users/${targetUserId}\\?revoked=1$`),
			);

			const revoked = await bearerApi.get(
				`/api/site/${harness.siteId}/assets/library`,
			);
			expect(revoked.status()).toBe(401);
			expect(revoked.headers()["www-authenticate"]).toContain("Bearer");

			await bearerApi.dispose();
			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});
});
