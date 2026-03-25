import { expect, test } from "@playwright/test";
import { access, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { tmpdir } from "node:os";

import {
	cleanupHarness,
	createAuthenticatedPage,
	createSite,
	createMembership,
	createUser,
	runCommand,
	setupHarness,
} from "./support";
import { defaultTimeout } from "./global_setup";

const gitEnv = {
	...process.env,
	GIT_AUTHOR_NAME: "Playwright",
	GIT_AUTHOR_EMAIL: "playwright@example.com",
	GIT_COMMITTER_NAME: "Playwright",
	GIT_COMMITTER_EMAIL: "playwright@example.com",
};

async function createThemeRepo(themeContent: string): Promise<string> {
	const repoRoot = await mkdtemp(path.join(tmpdir(), "websites-theme-repo-"));
	await runCommand("git", ["init", "-b", "main"], { cwd: repoRoot, env: gitEnv });
	await writeFile(path.join(repoRoot, "theme.txt"), themeContent);
	await runCommand("git", ["add", "theme.txt"], { cwd: repoRoot, env: gitEnv });
	await runCommand("git", ["commit", "-m", "initial theme"], {
		cwd: repoRoot,
		env: gitEnv,
	});
	return repoRoot;
}

async function updateThemeRepo(
	repoRoot: string,
	themeContent: string,
	message: string,
): Promise<void> {
	await writeFile(path.join(repoRoot, "theme.txt"), themeContent);
	await runCommand("git", ["add", "theme.txt"], { cwd: repoRoot, env: gitEnv });
	await runCommand("git", ["commit", "-m", message], {
		cwd: repoRoot,
		env: gitEnv,
	});
}

test.describe("theme registry admin UI", () => {
	test.setTimeout(defaultTimeout);

	test("installs a theme from a git repo and exposes it in the site UI", async ({
		browser,
	}) => {
		const harness = await setupHarness();
		const repoRoot = await createThemeRepo("version-one");

		try {
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"theme-ui-admin",
				true,
			);

			try {
				await page.goto(`${harness.baseUrl}/admin/themes`, {
					waitUntil: "domcontentloaded",
				});
				await page.getByLabel("Repository URL").fill(repoRoot);
				await page.getByLabel("Theme Slug").fill("sample-theme");
				await page.getByRole("button", { name: "Install theme" }).click();

				await expect(page).toHaveURL(
					`${harness.baseUrl}/admin/themes?installed=sample-theme`,
				);
				await expect(page.locator(".message--toast")).toContainText(
					"Theme installed.",
				);

				const row = page.locator("tbody tr").filter({ hasText: "sample-theme" });
				await expect(row).toContainText(repoRoot);
				await expect(row).toContainText("Installed");

				await page.goto(`${harness.baseUrl}/admin/sites/new`, {
					waitUntil: "domcontentloaded",
				});
				await expect(page.locator("#template_name")).toContainText(
					"sample-theme",
				);
			} finally {
				await context.close();
			}
		} finally {
			await rm(repoRoot, { recursive: true, force: true });
			await cleanupHarness(harness);
		}
	});

	test("updates a theme from the managed git repo", async ({ browser }) => {
		const harness = await setupHarness();
		const repoRoot = await createThemeRepo("version-one");

		try {
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"theme-ui-admin",
				true,
			);

			try {
				await page.goto(`${harness.baseUrl}/admin/themes`, {
					waitUntil: "domcontentloaded",
				});
				await page.getByLabel("Repository URL").fill(repoRoot);
				await page.getByLabel("Theme Slug").fill("sample-theme");
				await page.getByRole("button", { name: "Install theme" }).click();
				await expect(page).toHaveURL(
					`${harness.baseUrl}/admin/themes?installed=sample-theme`,
				);

				await updateThemeRepo(repoRoot, "version-two", "update theme");

				await page.getByRole("button", { name: "Update" }).click();
				await expect(page).toHaveURL(
					`${harness.baseUrl}/admin/themes?updated=sample-theme`,
				);
				await expect(page.locator(".message--toast")).toContainText(
					"Theme updated.",
				);

				const installedTheme = path.join(
					harness.tempRoot,
					"site_templates",
					"sample-theme",
					"theme.txt",
				);
				await expect(await pathExists(installedTheme)).toBe(true);
				await expect(await readText(installedTheme)).toBe("version-two");
			} finally {
				await context.close();
			}
		} finally {
			await rm(repoRoot, { recursive: true, force: true });
			await cleanupHarness(harness);
		}
	});

	test("blocks deleting a theme that is still in use", async ({ browser }) => {
		const harness = await setupHarness();
		const repoRoot = await createThemeRepo("version-one");

		try {
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"theme-ui-admin",
				true,
			);

			try {
				await page.goto(`${harness.baseUrl}/admin/themes`, {
					waitUntil: "domcontentloaded",
				});
				await page.getByLabel("Repository URL").fill(repoRoot);
				await page.getByLabel("Theme Slug").fill("sample-theme");
				await page.getByRole("button", { name: "Install theme" }).click();
				await expect(page).toHaveURL(
					`${harness.baseUrl}/admin/themes?installed=sample-theme`,
				);

				const siteId = await createSite(harness, {
					shortName: "theme-site",
					fullTitle: "Theme Site",
					templateName: "sample-theme",
				});

				const ownerSubject = "theme-site-owner";
				const ownerUserId = await createUser(harness, ownerSubject);
				await createMembership(harness, ownerUserId, "owner", siteId);
				const ownerSession = await createAuthenticatedPage(
					browser,
					harness,
					ownerSubject,
				);
				try {
					await ownerSession.page.goto(
						`${harness.baseUrl}/admin/site/${siteId}/settings`,
						{
							waitUntil: "domcontentloaded",
						},
					);
					await expect(ownerSession.page.locator("#template_name")).toContainText(
						"sample-theme",
					);
				} finally {
					await ownerSession.context.close();
				}

				await page.goto(`${harness.baseUrl}/admin/themes`, {
					waitUntil: "domcontentloaded",
				});
				await page.getByRole("button", { name: "Delete" }).click();

				await expect(page).toHaveURL(
					`${harness.baseUrl}/admin/themes/sample-theme/delete`,
				);
				await expect(page.locator("body")).toContainText(
					"sample-theme is still used by 1 site(s)",
				);

				await page.goto(`${harness.baseUrl}/admin/themes`, {
					waitUntil: "domcontentloaded",
				});
				await expect(
					page.locator("tbody tr").filter({ hasText: "sample-theme" }),
				).toBeVisible();
			} finally {
				await context.close();
			}
		} finally {
			await rm(repoRoot, { recursive: true, force: true });
			await cleanupHarness(harness);
		}
	});
});

async function pathExists(filePath: string): Promise<boolean> {
	try {
		await access(filePath);
		return true;
	} catch {
		return false;
	}
}

async function readText(filePath: string): Promise<string> {
	return readFile(filePath, "utf8");
}
