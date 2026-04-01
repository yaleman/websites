import { expect, test } from "@playwright/test";

import {
	addMembership,
	cleanupHarness,
	createAssetWithThumbnail,
	createAuthenticatedPage,
	createContent,
	createTag,
	createUser,
	seedSession,
	setupHarness,
	tinyPngBytes,
} from "./support";
import { defaultTimeout } from "./global_setup";

async function uploadAssetFromPage(
	page: import("@playwright/test").Page,
	harness: { port: number; siteId: string },
	filename: string,
) {
	await page.goto(
		`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets/new`,
		{ waitUntil: "domcontentloaded" },
	);
	await page.locator('input[type="file"]').setInputFiles({
		name: filename,
		mimeType: "image/png",
		buffer: Buffer.from(tinyPngBytes),
	});
	await Promise.all([
		page.waitForURL(
			`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets`,
		),
		page.getByRole("button", { name: "Upload", exact: true }).click(),
	]);
}

test.describe("content new editor", () => {
	test.setTimeout(defaultTimeout);

	test("renders TipTap editor", async ({ browser }) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "test-user");
			await addMembership(harness, userId, "owner");
			await createTag(harness, "news");
			const asset = {
				originalFilename: "test-image.png",
				storageBasename: "test-image.png",
				thumbnailFilename: "test-image_thumb.png",
			};
			await createAssetWithThumbnail(harness, asset);
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"test-user",
			);
			const consoleErrors: string[] = [];
			page.on("console", (message) => {
				if (message.type() === "error") {
					consoleErrors.push(message.text());
				}
			});
			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
				{ waitUntil: "domcontentloaded" },
			);

			await expect(page.locator("#editor")).toBeVisible();
			await page.locator(".ProseMirror").first().waitFor({ state: "visible" });
			await expect(page.locator("#page_content")).toBeHidden();
			await expect(page.locator("#tags")).toBeVisible();
			await expect(
				page.locator('#tag-suggestions option[value="news"]'),
			).toBeAttached();
			await page.locator("#tags").fill("news");
			await page.locator("#tags").press("Enter");
			await expect(page.locator('[data-tag-chip="news"]')).toBeVisible();
			await page.getByRole("button", { name: "Image" }).click();
			const modal = page.getByRole("dialog", { name: "Insert image" });
			await expect(modal).toBeVisible();
			const assetCard = modal.locator(".asset-card", {
				hasText: asset.originalFilename,
			});
			await expect(assetCard).toBeVisible();
			await assetCard.click();
			await modal.getByLabel("Alt text").fill("Test image");
			await modal.getByRole("button", { name: "Insert image" }).click();
			await expect(modal).toBeHidden();
			await page.getByRole("button", { name: "Image" }).click();
			await expect(modal).toBeVisible();
			await modal.getByPlaceholder("Search by filename").fill("test-image");
			await modal.getByRole("button", { name: "Cancel" }).click();
			await expect(modal).toBeHidden();
			await page.getByRole("button", { name: "Image" }).click();
			await expect(modal).toBeVisible();
			await expect(modal.getByPlaceholder("Search by filename")).toHaveValue(
				"",
			);
			await modal.getByRole("button", { name: "Cancel" }).click();
			await expect(modal).toBeHidden();
			await expect(
				page.locator(
					`#editor .ProseMirror a[href="/media/images/${asset.storageBasename}"] img[src="/media/images/${asset.thumbnailFilename}"]`,
				),
			).toBeVisible();
			const boldButton = page.getByRole("button", { name: "Bold" });
			const italicButton = page.getByRole("button", { name: "Italic" });
			const h2Button = page.getByRole("button", { name: "H2" });
			const h3Button = page.getByRole("button", { name: "H3" });
			const bulletButton = page.getByRole("button", { name: "Bullets" });
			const numberedButton = page.getByRole("button", { name: "Numbered" });
			const quoteButton = page.getByRole("button", { name: "Quote" });
			expect(
				consoleErrors.filter((message) => message.includes("contentMatchAt")),
			).toHaveLength(0);
			await page.locator(".ProseMirror").click();
			await page.keyboard.type("Preview check");
			await expect(boldButton).toHaveAttribute("aria-pressed", "false");
			await expect(page.locator("[data-editor-preview]")).toBeHidden();

			// TODO not currently tested as it's being removed/reworked
			// await page.getByRole("button", { name: "Preview" }).click();
			// await expect(page.locator("[data-editor-preview]")).toBeVisible();
			// await expect(page.locator("[data-editor-preview-body]")).toContainText(
			// 	"Preview check",
			// );

			await expect(page.locator("[data-editor-source-panel]")).toBeHidden();
			await page.getByRole("button", { name: "Markdown" }).click();
			await expect(page.locator("[data-editor-source-panel]")).toBeVisible();
			await expect(page.locator("#page_content")).toBeVisible();
			await page
				.locator("#page_content")
				.fill(
					"Plain intro\n\n**Bold source**\n\n## Raw heading\n\n### Nested heading\n\n- Bullet line\n\n1. Numbered line\n\n> Quoted line",
				);
			await expect(page.locator("[data-editor-preview-body]")).toContainText(
				"Raw heading",
			);
			await expect(page.locator("[data-editor-preview-body]")).toContainText(
				"Nested heading",
			);
			await expect(page.locator(".ProseMirror")).toContainText("Bold source");
			await expect(page.locator(".ProseMirror")).toContainText("Raw heading");
			await expect(page.locator(".ProseMirror")).toContainText("Nested heading");
			await expect(page.locator(".ProseMirror")).toContainText("Bullet line");
			await expect(page.locator(".ProseMirror")).toContainText("Numbered line");
			await expect(page.locator(".ProseMirror")).toContainText("Quoted line");

			const editorHeadingSizes = await page.evaluate(() => {
				const prose = document.querySelector(".ProseMirror");
				if (!(prose instanceof HTMLElement)) {
					throw new Error("Editor surface missing");
				}

				const h2 = prose.querySelector("h2");
				const h3 = prose.querySelector("h3");
				if (!(h2 instanceof HTMLElement) || !(h3 instanceof HTMLElement)) {
					throw new Error("Editor headings missing");
				}

				return {
					h2FontSize: Number.parseFloat(window.getComputedStyle(h2).fontSize),
					h3FontSize: Number.parseFloat(window.getComputedStyle(h3).fontSize),
				};
			});
			expect(editorHeadingSizes.h2FontSize).toBeGreaterThan(
				editorHeadingSizes.h3FontSize,
			);

			const previewHeadingSizes = await page.evaluate(() => {
				const preview = document.querySelector("[data-editor-preview-body]");
				if (!(preview instanceof HTMLElement)) {
					throw new Error("Preview surface missing");
				}

				const h2 = preview.querySelector("h2");
				const h3 = preview.querySelector("h3");
				if (!(h2 instanceof HTMLElement) || !(h3 instanceof HTMLElement)) {
					throw new Error("Preview headings missing");
				}

				return {
					h2FontSize: Number.parseFloat(window.getComputedStyle(h2).fontSize),
					h3FontSize: Number.parseFloat(window.getComputedStyle(h3).fontSize),
				};
			});
			expect(previewHeadingSizes.h2FontSize).toBeGreaterThan(
				previewHeadingSizes.h3FontSize,
			);

			await page.locator(".ProseMirror p").first().click();
			await page.keyboard.press("End");
			await boldButton.click();
			await expect(boldButton).toHaveAttribute("aria-pressed", "true");
			await page.keyboard.type(" tail");
			await expect(page.locator(".ProseMirror p").first()).toContainText(
				"Plain intro tail",
			);
			await expect(page.locator(".ProseMirror p").first().locator("strong")).toContainText(
				"tail",
			);

			await page.locator(".ProseMirror p").first().click();
			await page.keyboard.press("End");
			await page.keyboard.press("Shift+ArrowLeft");
			await page.keyboard.press("Shift+ArrowLeft");
			await page.keyboard.press("Shift+ArrowLeft");
			await page.keyboard.press("Shift+ArrowLeft");
			await italicButton.click();
			await expect(page.locator(".ProseMirror p").first().locator("em")).toContainText(
				"tail",
			);
			await expect(page.locator(".ProseMirror p").first()).toContainText(
				"Plain intro tail",
			);

			await page.locator(".ProseMirror p").first().locator("strong").click();
			await expect(boldButton).toHaveAttribute("aria-pressed", "true");
			await expect(h2Button).toHaveAttribute("aria-pressed", "false");
			await expect(h3Button).toHaveAttribute("aria-pressed", "false");

			await page.locator(".ProseMirror h2").click();
			await expect(h2Button).toHaveAttribute("aria-pressed", "true");
			await expect(h3Button).toHaveAttribute("aria-pressed", "false");

			await page.locator(".ProseMirror h3").click();
			await expect(h2Button).toHaveAttribute("aria-pressed", "false");
			await expect(h3Button).toHaveAttribute("aria-pressed", "true");

			await page.locator(".ProseMirror ul li").click();
			await expect(bulletButton).toHaveAttribute("aria-pressed", "true");
			await expect(numberedButton).toHaveAttribute("aria-pressed", "false");
			await expect(quoteButton).toHaveAttribute("aria-pressed", "false");

			await page.locator(".ProseMirror ol li").click();
			await expect(bulletButton).toHaveAttribute("aria-pressed", "false");
			await expect(numberedButton).toHaveAttribute("aria-pressed", "true");
			await expect(quoteButton).toHaveAttribute("aria-pressed", "false");

			await page.locator(".ProseMirror blockquote p").click();
			await expect(bulletButton).toHaveAttribute("aria-pressed", "false");
			await expect(numberedButton).toHaveAttribute("aria-pressed", "false");
			await expect(quoteButton).toHaveAttribute("aria-pressed", "true");

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/memberships`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(
				page.getByRole("heading", { name: "Memberships", exact: true }),
			).toBeVisible();
			await expect(page.locator('[aria-label="test-user"]')).toBeVisible();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("auto-fills and unlocks the new content slug on demand", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "slug-flow-user");
			await addMembership(harness, userId, "owner");
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"slug-flow-user",
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
				{ waitUntil: "domcontentloaded" },
			);

			await expect(page.locator("#slug")).not.toBeEditable();
			await page.locator("#title").fill("A Fresh Title");
			await expect(page.locator("#slug")).toHaveValue("a-fresh-title");

			const focusableOrder = await page.locator("form.editor-form").evaluate(
				(form) => {
					const visible = (element: HTMLElement) => {
						const style = window.getComputedStyle(element);
						return (
							style.display !== "none" &&
							style.visibility !== "hidden" &&
							!element.hidden
						);
					};

					return Array.from(
						form.querySelectorAll<HTMLElement>(
							"button, input, select, textarea, [contenteditable='true']",
						),
					)
						.filter(
							(element) =>
								visible(element) &&
								!element.closest("[hidden]") &&
								!(element as HTMLInputElement).disabled &&
								element.tabIndex >= 0,
						)
						.map(
							(element) =>
								element.id ||
								element.getAttribute("data-slug-reset") ||
								element.getAttribute("data-command") ||
								element.tagName.toLowerCase(),
						);
				},
			);
			expect(focusableOrder[focusableOrder.length - 1]).toBe("slug");

			await page.once("dialog", async (dialog) => {
				expect(dialog.message()).toContain(
					"Edit the slug manually? It will stop following the title until you reset it.",
				);
				await dialog.accept();
			});
			await page.locator("#slug").click();
			await expect(page.locator("#slug")).toBeEditable();
			await page.locator("#slug").fill("manual-slug");
			await expect(page.locator("#slug")).toHaveValue("manual-slug");

			await page.locator("#title").fill("Changed Title");
			await expect(page.locator("#slug")).toHaveValue("manual-slug");

			await page.getByRole("button", { name: "Use title slug" }).click();
			await expect(page.locator("#slug")).toHaveValue("changed-title");
			await page.locator("#title").fill("Another Title");
			await expect(page.locator("#slug")).toHaveValue("another-title");

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("refreshes the image picker after uploads from another tab", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "cross-tab-asset-refresh";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
				{ waitUntil: "domcontentloaded" },
			);

			const uploadPage = await context.newPage();
			await uploadAssetFromPage(uploadPage, harness, "fresh-cross-tab-image.png");
			await uploadPage.close();

			await page.bringToFront();
			await page.getByRole("button", { name: "Image" }).click();
			const modal = page.getByRole("dialog", { name: "Insert image" });
			await expect(modal).toBeVisible();
			await expect(
				modal.locator(".asset-card", {
					hasText: "fresh-cross-tab-image.png",
				}),
			).toBeVisible();

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("refreshes an open image picker after uploading from the modal link", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "modal-link-asset-refresh";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
				{ waitUntil: "domcontentloaded" },
			);

			await page.getByRole("button", { name: "Image" }).click();
			const modal = page.getByRole("dialog", { name: "Insert image" });
			await expect(modal).toBeVisible();

			const uploadLink = modal.getByRole("link", { name: "Upload image" });
			await expect(uploadLink).toHaveAttribute(
				"href",
				`/admin/site/${harness.siteId}/assets/new`,
			);
			await expect(uploadLink).toHaveAttribute("target", "_blank");
			await expect(uploadLink).toHaveAttribute("rel", "noopener noreferrer");
			await expect(
				modal.locator(".asset-card", {
					hasText: "modal-refresh-image.png",
				}),
			).toHaveCount(0);

			const [uploadPage] = await Promise.all([
				context.waitForEvent("page"),
				uploadLink.click(),
			]);
			await uploadPage.waitForURL(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/assets/new`,
			);
			await uploadAssetFromPage(uploadPage, harness, "modal-refresh-image.png");
			await uploadPage.close();

			await page.bringToFront();
			await expect(
				modal.locator(".asset-card", {
					hasText: "modal-refresh-image.png",
				}),
			).toBeVisible();

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("creates content and lands in the editor with a toast", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "create-content-user");
			await addMembership(harness, userId, "owner");
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"create-content-user",
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
				{ waitUntil: "domcontentloaded" },
			);

			await page.locator("#title").fill("Created from the new page");
			await expect(page.locator("#slug")).toHaveValue("created-from-the-new-page");
			await page.locator(".ProseMirror").click();
			await page.keyboard.type("Created body from the new page");

			await Promise.all([
				page.getByRole("button", { name: "Create content" }).click(),
				page.waitForURL(
					(newUrl) =>
						newUrl.pathname.match(
							new RegExp(
								`^/admin/site/${harness.siteId}/content/[0-9a-f-]+/edit$`,
							),
						) !== null,
				),
			]);

			const toast = page.locator(".message--toast");
			await expect(toast).toBeVisible();
			await expect(toast).toContainText("Content saved.");
			await expect(page.getByRole("button", { name: "Save content" })).toBeVisible();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("allows access without membership", async ({ browser }) => {
		const harness = await setupHarness();

		try {
			await createUser(harness, "intruder");
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"intruder",
			);
			const response = await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
				{ waitUntil: "domcontentloaded" },
			);

			expect(response).not.toBeNull();
			expect(response?.status()).toBe(401);
			await expect(page.locator("body")).toContainText(
				`missing membership for site ${harness.siteId}`,
			);
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("allows system admin access without membership", async ({ browser }) => {
		const harness = await setupHarness();

		try {
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"site-admin",
				true,
			);
			const response = await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
				{ waitUntil: "domcontentloaded" },
			);

			expect(response).not.toBeNull();
			expect(response?.status()).toBe(200);
			await expect(page.locator("#editor")).toBeVisible();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("adds an existing user to memberships from the autocomplete input", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const ownerId = await createUser(harness, "membership-owner");
			await addMembership(harness, ownerId, "owner");
			await createUser(harness, "membership-target", {
				email: "membership-target@example.com",
			});
			await seedSession(harness, "membership-target");
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"membership-owner",
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/memberships`,
				{ waitUntil: "domcontentloaded" },
			);

			await page.getByLabel("User").fill("membership-target");
			await expect(
				page.getByRole("option", {
					name: /membership-target membership-target@example.com/i,
				}),
			).toBeVisible();
			await page.getByLabel("User").fill("membership-target@example");
			await expect(page.getByRole("option", {
				name: /membership-target membership-target@example.com/i,
			})).toBeVisible();
			await page.getByLabel("User").press("Enter");
			await expect(page.getByLabel("User")).toHaveValue(
				"membership-target@example.com (membership-target)",
			);
			await page.getByRole("button", { name: "Add member" }).click();
			await expect(
				page.locator('[aria-label="membership-target"]'),
			).toBeVisible();
			await expect(
				page.getByText("membership-target@example.com"),
			).toBeVisible();

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("saves back to the source editor and shows a toast", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "editor-user");
			await addMembership(harness, userId, "owner");
			const contentId = await createContent(harness, {
				pageType: "page",
				title: "Existing page",
				slug: "existing-page",
				pageContent: "Initial body",
				creatorSub: "editor-user",
			});
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"editor-user",
			);
			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				{ waitUntil: "domcontentloaded" },
			);

			await page.getByRole("button", { name: "Markdown" }).click();
			await page.locator("#page_content").fill("Updated body from source mode");
			await page.getByRole("button", { name: "Save content" }).click();

			await expect(page).toHaveURL(
				new RegExp(`/admin/site/${harness.siteId}/content/${contentId}/edit`),
			);
			const toast = page.locator(".message--toast");
			await expect(toast).toBeVisible();
			await expect(toast).toContainText("Content saved.");
			await expect(page.locator("#page_content")).toHaveValue(
				"Updated body from source mode",
			);
			await expect(toast).toBeHidden({ timeout: 4000 });
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("updates tags from the source editor", async ({ browser }) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "tag-editor");
			await addMembership(harness, userId, "owner");
			await createTag(harness, "docs");
			await createTag(harness, "news");
			const contentId = await createContent(harness, {
				pageType: "page",
				title: "Tagged page",
				slug: "tagged-page",
				pageContent: "Initial body",
				creatorSub: "tag-editor",
			});
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"tag-editor",
			);
			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				{ waitUntil: "domcontentloaded" },
			);

			await page.locator("#tags").fill("docs");
			await page.locator("#tags").press("Enter");
			await page.locator("#tags").fill("news");
			await page.locator("#tags").press("Enter");
			await Promise.all([
				page.waitForResponse(
					(response) =>
						response.request().method() === "POST" &&
						response.url() ===
							`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				),
				page.getByRole("button", { name: "Save content" }).click(),
			]);
			await page.waitForLoadState("domcontentloaded");
			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				{ waitUntil: "domcontentloaded" },
			);

			await expect(page.locator('[data-tag-chip="docs"]')).toBeVisible();
			await expect(page.locator('[data-tag-chip="news"]')).toBeVisible();

			await page.locator("#tags").fill("guides");
			await page.locator("#tags").press("Enter");
			await page.locator('[data-tag-chip="docs"]').click();
			await page.locator('[data-tag-chip="news"]').click();
			await Promise.all([
				page.waitForResponse(
					(response) =>
						response.request().method() === "POST" &&
						response.url() ===
							`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				),
				page.getByRole("button", { name: "Save content" }).click(),
			]);
			await page.waitForLoadState("domcontentloaded");
			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				{ waitUntil: "domcontentloaded" },
			);
			await expect(page.locator('[data-tag-chip="guides"]')).toBeVisible();
			await expect(page.locator('[data-tag-chip="docs"]')).toHaveCount(0);
			await expect(page.locator('[data-tag-chip="news"]')).toHaveCount(0);
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("creates and deletes tags from the tags admin page", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "tag-admin");
			await addMembership(harness, userId, "owner");
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"tag-admin",
			);
			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/tags`,
				{ waitUntil: "domcontentloaded" },
			);

			await page.getByLabel("New tag").fill("release-notes");
			await page.getByRole("button", { name: "Create tag" }).click();
			await expect(
				page.getByRole("cell", { name: "release-notes" }),
			).toBeVisible();

			await page
				.locator("tr", {
					has: page.getByRole("cell", { name: "release-notes" }),
				})
				.getByRole("button", { name: "Delete" })
				.click();
			await expect(
				page.getByRole("cell", { name: "release-notes" }),
			).toHaveCount(0);
		} finally {
			await cleanupHarness(harness);
		}
	});
});

test.describe("user profile", () => {
	test.setTimeout(defaultTimeout);

	test("lets a user view their own profile details and memberships", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "profile-user");
			await addMembership(harness, userId, "author");
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"profile-user",
			);
			const response = await page.goto(
				`https://127.0.0.1:${harness.port}/admin/users/${userId}`,
				{ waitUntil: "domcontentloaded" },
			);

			expect(response).not.toBeNull();
			expect(response?.status()).toBe(200);
			await expect(
				page.getByRole("heading", { name: "User Profile: profile-user" }),
			).toBeVisible();
			await expect
				.poll(() => page.content())
				.toContain(`Database ID: ${userId}`);
			await expect(
				page.getByRole("cell", { name: "profile-user" }),
			).toBeVisible();
			await expect(page.getByRole("cell", { name: "No" })).toBeVisible();
			const membershipRow = page.getByRole("row", {
				name: /Test Site test Author/,
			});
			await expect(membershipRow).toBeVisible();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("lets a system admin view another user's profile", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const targetUserId = await createUser(harness, "target-user");
			await addMembership(harness, targetUserId, "viewer");
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"global-admin",
				true,
			);
			const response = await page.goto(
				`https://127.0.0.1:${harness.port}/admin/users/${targetUserId}`,
				{ waitUntil: "domcontentloaded" },
			);

			expect(response).not.toBeNull();
			expect(response?.status()).toBe(200);
			await expect(
				page.getByRole("heading", { name: "User Profile: target-user" }),
			).toBeVisible();
			await expect(
				page.getByRole("cell", { name: "target-user" }),
			).toBeVisible();
			await expect(
				page.getByRole("row", { name: /Test Site test Viewer/ }),
			).toBeVisible();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("hides site memberships on an admin user's profile", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const targetUserId = await createUser(harness, "admin-profile-user", {
				admin: true,
			});
			await addMembership(harness, targetUserId, "viewer");
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"admin-profile-user",
			);
			const response = await page.goto(
				`https://127.0.0.1:${harness.port}/admin/users/${targetUserId}`,
				{ waitUntil: "domcontentloaded" },
			);

			expect(response).not.toBeNull();
			expect(response?.status()).toBe(200);
			await expect(
				page.getByRole("heading", { name: "User Profile: admin-profile-user" }),
			).toBeVisible();
			await expect(page.getByRole("heading", { name: "Site Memberships" })).toHaveCount(
				0,
			);
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("blocks a non-admin user from viewing another user's profile", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			await createUser(harness, "viewer-user");
			const targetUserId = await createUser(harness, "private-user");
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"viewer-user",
			);
			const response = await page.goto(
				`https://127.0.0.1:${harness.port}/admin/users/${targetUserId}`,
				{ waitUntil: "domcontentloaded" },
			);

			expect(response).not.toBeNull();
			expect(response?.status()).toBe(401);
			await expect(page.locator("body")).toContainText(
				"cannot view another user's profile",
			);
		} finally {
			await cleanupHarness(harness);
		}
	});
});
