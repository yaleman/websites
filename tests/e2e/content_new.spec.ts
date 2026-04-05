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

async function selectTextInEditor(
	page: import("@playwright/test").Page,
	selector: string,
	targetText: string,
) {
	await page.locator(selector).evaluate((element, expectedText) => {
		const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
		let currentNode: Node | null = walker.nextNode();
		while (currentNode) {
			const text = currentNode.textContent ?? "";
			const start = text.indexOf(expectedText);
			if (start >= 0) {
				const range = document.createRange();
				range.setStart(currentNode, start);
				range.setEnd(currentNode, start + expectedText.length);
				const selection = window.getSelection();
				if (!selection) {
					throw new Error("Selection unavailable");
				}
				selection.removeAllRanges();
				selection.addRange(range);
				return;
			}
			currentNode = walker.nextNode();
		}

		throw new Error(`Could not find text "${expectedText}" in ${selector}`);
	}, targetText);
}

async function placeCaretInEditor(
	page: import("@playwright/test").Page,
	selector: string,
	targetText: string,
) {
	await page.locator(selector).evaluate((element, expectedText) => {
		const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
		let currentNode: Node | null = walker.nextNode();
		while (currentNode) {
			const text = currentNode.textContent ?? "";
			const offset = text.indexOf(expectedText);
			if (offset >= 0) {
				const range = document.createRange();
				range.setStart(currentNode, offset + expectedText.length);
				range.collapse(true);
				const selection = window.getSelection();
				if (!selection) {
					throw new Error("Selection unavailable");
				}
				selection.removeAllRanges();
				selection.addRange(range);
				return;
			}
			currentNode = walker.nextNode();
		}

		throw new Error(`Could not place caret after "${expectedText}" in ${selector}`);
	}, targetText);
}

async function selectTableCells(
	page: import("@playwright/test").Page,
	firstSelector: string,
	secondSelector: string,
) {
	await page.locator(firstSelector).click();
	await page.locator(secondSelector).click({ modifiers: ["Shift"] });
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
			const sizeSelect = page.getByRole("combobox", { name: "Size" });
			const bulletButton = page.getByRole("button", { name: "Bullets" });
			const numberedButton = page.getByRole("button", { name: "Numbered" });
			const quoteButton = page.getByRole("button", { name: "Quote" });
			expect(
				consoleErrors.filter((message) => message.includes("contentMatchAt")),
			).toHaveLength(0);
			await expect(sizeSelect).toBeVisible();
			const sizeOptions = await sizeSelect.locator("option").evaluateAll((options) =>
				options.map((option) => ({
					value: (option as HTMLOptionElement).value,
					label: option.textContent?.trim() ?? "",
				})),
			);
			expect(sizeOptions).toEqual([
				{ value: "normal", label: "Normal" },
				{ value: "h1", label: "H1" },
				{ value: "h2", label: "H2" },
				{ value: "h3", label: "H3" },
				{ value: "h4", label: "H4" },
				{ value: "h5", label: "H5" },
				{ value: "h6", label: "H6" },
			]);
			await page.locator(".ProseMirror").click();
			await page.keyboard.type("Preview check");
			await expect(boldButton).toHaveAttribute("aria-pressed", "false");
			await expect(sizeSelect).toHaveValue("normal");
			await expect(page.locator("[data-editor-preview]")).toBeHidden();

			await sizeSelect.selectOption("h1");
			await expect(sizeSelect).toHaveValue("h1");
			await expect(
				page.locator(".ProseMirror h1").filter({ hasText: "Preview check" }),
			).toBeVisible();

			await sizeSelect.selectOption("h2");
			await expect(sizeSelect).toHaveValue("h2");
			await expect(
				page.locator(".ProseMirror h2").filter({ hasText: "Preview check" }),
			).toBeVisible();

			await sizeSelect.selectOption("h3");
			await expect(sizeSelect).toHaveValue("h3");
			await expect(
				page.locator(".ProseMirror h3").filter({ hasText: "Preview check" }),
			).toBeVisible();

			await sizeSelect.selectOption("h6");
			await expect(sizeSelect).toHaveValue("h6");
			await expect(
				page.locator(".ProseMirror h6").filter({ hasText: "Preview check" }),
			).toBeVisible();

			await sizeSelect.selectOption("normal");
			await expect(sizeSelect).toHaveValue("normal");
			await expect(
				page.locator(".ProseMirror p").filter({ hasText: "Preview check" }),
			).toBeVisible();

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
					"Plain intro\n\n# Hero heading\n\n## Raw heading\n\n### Nested heading\n\n#### Minor heading\n\n##### Compact heading\n\n###### Tiny heading\n\n- Bullet line\n\n1. Numbered line\n\n> Quoted line",
				);
			await page.locator(".ProseMirror p").first().click();
			await page.keyboard.type("!");
			const markdownRoundTrip = await page.locator("#page_content").inputValue();
			expect(markdownRoundTrip).toContain("# Hero heading");
			expect(markdownRoundTrip).toContain("## Raw heading");
			expect(markdownRoundTrip).toContain("### Nested heading");
			expect(markdownRoundTrip).toContain("#### Minor heading");
			expect(markdownRoundTrip).toContain("##### Compact heading");
			expect(markdownRoundTrip).toContain("###### Tiny heading");
			await expect(page.locator("[data-editor-preview-body]")).toContainText(
				"Hero heading",
			);
			await expect(page.locator("[data-editor-preview-body]")).toContainText(
				"Raw heading",
			);
			await expect(page.locator("[data-editor-preview-body]")).toContainText(
				"Nested heading",
			);
			await expect(page.locator("[data-editor-preview-body]")).toContainText(
				"Minor heading",
			);
			await expect(page.locator("[data-editor-preview-body]")).toContainText(
				"Compact heading",
			);
			await expect(page.locator("[data-editor-preview-body]")).toContainText(
				"Tiny heading",
			);
			await expect(page.locator(".ProseMirror")).toContainText("Raw heading");
			await expect(page.locator(".ProseMirror")).toContainText("Nested heading");
			await expect(page.locator(".ProseMirror")).toContainText("Hero heading");
			await expect(page.locator(".ProseMirror")).toContainText("Minor heading");
			await expect(page.locator(".ProseMirror")).toContainText("Compact heading");
			await expect(page.locator(".ProseMirror")).toContainText("Tiny heading");
			await expect(page.locator(".ProseMirror")).toContainText("Bullet line");
			await expect(page.locator(".ProseMirror")).toContainText("Numbered line");
			await expect(page.locator(".ProseMirror")).toContainText("Quoted line");
			const listStyles = await page.evaluate(() => {
				const prose = document.querySelector(".ProseMirror");
				const preview = document.querySelector("[data-editor-preview-body]");
				if (!(prose instanceof HTMLElement) || !(preview instanceof HTMLElement)) {
					throw new Error("Editor surfaces missing");
				}

				const editorBulletList = prose.querySelector("ul");
				const editorNumberedList = prose.querySelector("ol");
				const previewBulletList = preview.querySelector("ul");
				const previewNumberedList = preview.querySelector("ol");
				if (
					!(editorBulletList instanceof HTMLElement) ||
					!(editorNumberedList instanceof HTMLElement) ||
					!(previewBulletList instanceof HTMLElement) ||
					!(previewNumberedList instanceof HTMLElement)
				) {
					throw new Error("Rendered lists missing");
				}

				return {
					editorBullet: window.getComputedStyle(editorBulletList).listStyleType,
					editorNumbered: window.getComputedStyle(editorNumberedList).listStyleType,
					previewBullet: window.getComputedStyle(previewBulletList).listStyleType,
					previewNumbered:
						window.getComputedStyle(previewNumberedList).listStyleType,
				};
			});
			expect(listStyles.editorBullet).toBe("disc");
			expect(listStyles.editorNumbered).toBe("decimal");
			expect(listStyles.previewBullet).toBe("disc");
			expect(listStyles.previewNumbered).toBe("decimal");
			const blockquoteStyles = await page.evaluate(() => {
				const prose = document.querySelector(".ProseMirror");
				const preview = document.querySelector("[data-editor-preview-body]");
				if (!(prose instanceof HTMLElement) || !(preview instanceof HTMLElement)) {
					throw new Error("Editor surfaces missing");
				}

				const editorQuote = prose.querySelector("blockquote");
				const previewQuote = preview.querySelector("blockquote");
				if (
					!(editorQuote instanceof HTMLElement) ||
					!(previewQuote instanceof HTMLElement)
				) {
					throw new Error("Rendered blockquotes missing");
				}

				return {
					editorBorderLeftWidth:
						window.getComputedStyle(editorQuote).borderLeftWidth,
					editorFontStyle: window.getComputedStyle(editorQuote).fontStyle,
					previewBorderLeftWidth:
						window.getComputedStyle(previewQuote).borderLeftWidth,
					previewFontStyle: window.getComputedStyle(previewQuote).fontStyle,
				};
			});
			expect(blockquoteStyles.editorBorderLeftWidth).toBe("4px");
			expect(blockquoteStyles.editorFontStyle).toBe("italic");
			expect(blockquoteStyles.previewBorderLeftWidth).toBe("4px");
			expect(blockquoteStyles.previewFontStyle).toBe("italic");

			const editorHeadingSizes = await page.evaluate(() => {
				const prose = document.querySelector(".ProseMirror");
				if (!(prose instanceof HTMLElement)) {
					throw new Error("Editor surface missing");
				}

				const headings = Array.from({ length: 6 }, (_, index) =>
					prose.querySelector(`h${index + 1}`),
				);
				if (headings.some((heading) => !(heading instanceof HTMLElement))) {
					throw new Error("Editor headings missing");
				}

				return headings.map((heading) =>
					Number.parseFloat(
						window.getComputedStyle(heading as HTMLElement).fontSize,
					),
				);
			});
			expect(editorHeadingSizes[0]).toBeGreaterThan(editorHeadingSizes[1]);
			expect(editorHeadingSizes[1]).toBeGreaterThan(editorHeadingSizes[2]);
			expect(editorHeadingSizes[2]).toBeGreaterThan(editorHeadingSizes[3]);
			expect(editorHeadingSizes[3]).toBeGreaterThan(editorHeadingSizes[4]);
			expect(editorHeadingSizes[4]).toBeGreaterThan(editorHeadingSizes[5]);

			const previewHeadingSizes = await page.evaluate(() => {
				const preview = document.querySelector("[data-editor-preview-body]");
				if (!(preview instanceof HTMLElement)) {
					throw new Error("Preview surface missing");
				}

				const headings = Array.from({ length: 6 }, (_, index) =>
					preview.querySelector(`h${index + 1}`),
				);
				if (headings.some((heading) => !(heading instanceof HTMLElement))) {
					throw new Error("Preview headings missing");
				}

				return headings.map((heading) =>
					Number.parseFloat(
						window.getComputedStyle(heading as HTMLElement).fontSize,
					),
				);
			});
			expect(previewHeadingSizes[0]).toBeGreaterThan(previewHeadingSizes[1]);
			expect(previewHeadingSizes[1]).toBeGreaterThan(previewHeadingSizes[2]);
			expect(previewHeadingSizes[2]).toBeGreaterThan(previewHeadingSizes[3]);
			expect(previewHeadingSizes[3]).toBeGreaterThan(previewHeadingSizes[4]);
			expect(previewHeadingSizes[4]).toBeGreaterThan(previewHeadingSizes[5]);

			await page.locator(".ProseMirror p").first().click();
			await expect(sizeSelect).toHaveValue("normal");
			await page.keyboard.press("End");
			await boldButton.click();
			await expect(boldButton).toHaveAttribute("aria-pressed", "true");
			await page.keyboard.type(" tail");
			await expect(page.locator(".ProseMirror p").first()).toContainText(
				"Plain intro! tail",
			);
			await expect(page.locator(".ProseMirror p").first().locator("strong")).toContainText(
				"tail",
			);

			await page.locator(".ProseMirror p").first().click();
			await page.keyboard.press("End");
			await selectTextInEditor(page, "#editor .ProseMirror > p:first-of-type", "tail");
			await italicButton.click();
			const tailFormatting = await page.locator(".ProseMirror p").first().evaluate(
				(paragraph) => {
					const walker = document.createTreeWalker(paragraph, NodeFilter.SHOW_TEXT);
					let currentNode: Node | null = walker.nextNode();
					while (currentNode) {
						if ((currentNode.textContent ?? "").includes("tail")) {
							let currentElement = currentNode.parentElement;
							let hasStrong = false;
							let hasEmphasis = false;
							while (currentElement && currentElement !== paragraph) {
								hasStrong ||= currentElement.tagName === "STRONG";
								hasEmphasis ||= currentElement.tagName === "EM";
								currentElement = currentElement.parentElement;
							}
							return { hasStrong, hasEmphasis };
						}
						currentNode = walker.nextNode();
					}
					return { hasStrong: false, hasEmphasis: false };
				},
			);
			expect(tailFormatting).toEqual({ hasStrong: true, hasEmphasis: true });
			await expect(page.locator(".ProseMirror p").first()).toContainText(
				"Plain intro! tail",
			);

			await page.locator(".ProseMirror p").first().locator("strong").click();
			await expect(boldButton).toHaveAttribute("aria-pressed", "true");
			await expect(sizeSelect).toHaveValue("normal");

			await page.locator(".ProseMirror h1").click();
			await expect(sizeSelect).toHaveValue("h1");

			await page.locator(".ProseMirror h2").click();
			await expect(sizeSelect).toHaveValue("h2");

			await page.locator(".ProseMirror h3").click();
			await expect(sizeSelect).toHaveValue("h3");

			await page.locator(".ProseMirror h6").click();
			await expect(sizeSelect).toHaveValue("h6");

			await page.locator(".ProseMirror ul li").click();
			await expect(sizeSelect).toHaveValue("normal");
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

	test("applies toolbar features through the visible controls", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const userId = await createUser(harness, "toolbar-audit-user");
			await addMembership(harness, userId, "owner");
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				"toolbar-audit-user",
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
				{ waitUntil: "domcontentloaded" },
			);

			const proseMirror = page.locator(".ProseMirror");
			const boldButton = page.getByRole("button", { name: "Bold" });
			const italicButton = page.getByRole("button", { name: "Italic" });
			const codeButton = page.getByRole("button", { name: "Code" });
			const linkButton = page.getByRole("button", { name: "Link" });
			const bulletButton = page.getByRole("button", { name: "Bullets" });
			const numberedButton = page.getByRole("button", { name: "Numbered" });
			const quoteButton = page.getByRole("button", { name: "Quote" });
			const sourceButton = page.getByRole("button", { name: "Markdown" });

			await proseMirror.click();
			await page.keyboard.type("Alpha Beta Gamma Delta");
			await sourceButton.click();
			await expect(page.locator("[data-editor-source-panel]")).toBeVisible();

			await selectTextInEditor(page, ".ProseMirror p", "Alpha");
			await boldButton.click();
			await expect(page.locator(".ProseMirror strong")).toContainText("Alpha");

			await selectTextInEditor(page, ".ProseMirror p", "Beta");
			await italicButton.click();
			await expect(page.locator(".ProseMirror em")).toContainText("Beta");

			await selectTextInEditor(page, ".ProseMirror p", "Gamma");
			await codeButton.click();
			await expect(page.locator(".ProseMirror code")).toContainText("Gamma");

			page.once("dialog", async (dialog) => {
				expect(dialog.type()).toBe("prompt");
				await dialog.accept("https://example.com/docs");
			});
			await selectTextInEditor(page, ".ProseMirror p", "Delta");
			await linkButton.click();
			await expect(
				page.locator('.ProseMirror a[href="https://example.com/docs"]'),
			).toContainText("Delta");

			const richTextMarkdown = await page.locator("#page_content").inputValue();
			expect(richTextMarkdown).toContain("**Alpha**");
			expect(richTextMarkdown).toMatch(/(\*|_)Beta(\*|_)/);
			expect(richTextMarkdown).toContain("`Gamma`");
			expect(richTextMarkdown).toContain("[Delta](https://example.com/docs)");

			const inlineStyles = await page.evaluate(() => {
				const prose = document.querySelector(".ProseMirror");
				const preview = document.querySelector("[data-editor-preview-body]");
				if (!(prose instanceof HTMLElement) || !(preview instanceof HTMLElement)) {
					throw new Error("Editor surfaces missing");
				}

				const editorLink = prose.querySelector("a");
				const editorCode = prose.querySelector("code");
				const previewLink = preview.querySelector("a");
				const previewCode = preview.querySelector("code");
				if (
					!(editorLink instanceof HTMLElement) ||
					!(editorCode instanceof HTMLElement) ||
					!(previewLink instanceof HTMLElement) ||
					!(previewCode instanceof HTMLElement)
				) {
					throw new Error("Rendered inline formatting missing");
				}

				return {
					editorLinkDecoration:
						window.getComputedStyle(editorLink).textDecorationLine,
					previewLinkDecoration:
						window.getComputedStyle(previewLink).textDecorationLine,
					editorCodeBackground:
						window.getComputedStyle(editorCode).backgroundColor,
					previewCodeBackground:
						window.getComputedStyle(previewCode).backgroundColor,
				};
			});
			expect(inlineStyles.editorLinkDecoration).toContain("underline");
			expect(inlineStyles.previewLinkDecoration).toContain("underline");
			expect(inlineStyles.editorCodeBackground).not.toBe("rgba(0, 0, 0, 0)");
			expect(inlineStyles.previewCodeBackground).not.toBe("rgba(0, 0, 0, 0)");

			await page.locator("#page_content").fill("First item\n\nSecond item\n\nThird quote");
			await expect(proseMirror).toContainText("First item");
			await expect(proseMirror).toContainText("Second item");
			await expect(proseMirror).toContainText("Third quote");

			await page.locator(".ProseMirror p", { hasText: "First item" }).click();
			await bulletButton.click();
			await expect(page.locator(".ProseMirror ul li")).toContainText("First item");

			await page.locator(".ProseMirror p", { hasText: "Second item" }).click();
			await numberedButton.click();
			await expect(page.locator(".ProseMirror ol li")).toContainText("Second item");

			await page.locator(".ProseMirror p", { hasText: "Third quote" }).click();
			await quoteButton.click();
			await expect(page.locator(".ProseMirror blockquote p")).toContainText(
				"Third quote",
			);

			const structuralMarkdown = await page.locator("#page_content").inputValue();
			expect(structuralMarkdown).toContain("- First item");
			expect(structuralMarkdown).toContain("1. Second item");
			expect(structuralMarkdown).toContain("> Third quote");

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("creates and edits markdown tables from the toolbar", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "table-toolbar-user";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");
			const contentId = await createContent(harness, {
				pageType: "page",
				title: "Table editing page",
				slug: "table-editing-page",
				pageContent: "Intro paragraph",
				creatorSub: subject,
			});
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				{ waitUntil: "domcontentloaded" },
			);

			const tableButton = page.getByRole("button", { name: "Table" });
			const rowBeforeButton = page.getByRole("button", { name: "Row before" });
			const rowAfterButton = page.getByRole("button", { name: "Row after" });
			const deleteRowButton = page.getByRole("button", { name: "Delete row" });
			const columnBeforeButton = page.getByRole("button", {
				name: "Column before",
			});
			const columnAfterButton = page.getByRole("button", {
				name: "Column after",
			});
			const deleteColumnButton = page.getByRole("button", {
				name: "Delete column",
			});
			const mergeCellsButton = page.getByRole("button", { name: "Merge cells" });
			const splitCellButton = page.getByRole("button", { name: "Split cell" });
			const headerRowButton = page.getByRole("button", { name: "Header row" });
			const headerColumnButton = page.getByRole("button", {
				name: "Header column",
			});
			const headerCellButton = page.getByRole("button", { name: "Header cell" });

			await expect(page.locator("[data-table-controls]")).toBeHidden();
			await tableButton.click();
			await expect(page.locator("[data-table-controls]")).toBeVisible();
			await expect(page.locator(".ProseMirror table")).toBeVisible();
			await expect(page.locator(".ProseMirror table tr")).toHaveCount(2);
			await expect(page.locator(".ProseMirror th")).toHaveCount(2);

			await page.locator(".ProseMirror th").nth(0).click();
			await page.keyboard.type("Name");
			await page.keyboard.press("Tab");
			await page.keyboard.type("Role");
			await page.keyboard.press("Tab");
			await page.keyboard.type("Status");
			await page.keyboard.press("Tab");
			await page.keyboard.type("Alice");
			await page.keyboard.press("Tab");
			await page.keyboard.type("Writer");
			await page.keyboard.press("Tab");
			await page.keyboard.type("Active");
			await page.keyboard.press("Tab");
			await page.keyboard.type("Bob");
			await page.keyboard.press("Tab");
			await page.keyboard.type("Editor");
			await page.keyboard.press("Tab");
			await page.keyboard.type("Reviewing");

			const rowCountBeforeRowEdits = await page
				.locator(".ProseMirror table tr")
				.count();

			await page.locator(".ProseMirror table tr").nth(1).locator("td").nth(0).click();
			await rowBeforeButton.click();
			await expect(page.locator(".ProseMirror table tr")).toHaveCount(
				rowCountBeforeRowEdits + 1,
			);
			await deleteRowButton.click();
			await expect(page.locator(".ProseMirror table tr")).toHaveCount(
				rowCountBeforeRowEdits,
			);

			await page.locator(".ProseMirror table tr").nth(1).locator("td").nth(0).click();
			await rowAfterButton.click();
			await expect(page.locator(".ProseMirror table tr")).toHaveCount(
				rowCountBeforeRowEdits + 1,
			);
			await deleteRowButton.click();
			await expect(page.locator(".ProseMirror table tr")).toHaveCount(
				rowCountBeforeRowEdits,
			);

			const headerCellCountBeforeColumnEdits = await page
				.locator(".ProseMirror th")
				.count();

			await page.locator(".ProseMirror table tr").nth(1).locator("td").nth(1).click();
			await columnBeforeButton.click();
			await expect(page.locator(".ProseMirror th")).toHaveCount(
				headerCellCountBeforeColumnEdits + 1,
			);
			await deleteColumnButton.click();
			await expect(page.locator(".ProseMirror th")).toHaveCount(
				headerCellCountBeforeColumnEdits,
			);

			await page.locator(".ProseMirror table tr").nth(1).locator("td").nth(1).click();
			await columnAfterButton.click();
			await expect(page.locator(".ProseMirror th")).toHaveCount(
				headerCellCountBeforeColumnEdits + 1,
			);
			await deleteColumnButton.click();
			await expect(page.locator(".ProseMirror th")).toHaveCount(
				headerCellCountBeforeColumnEdits,
			);

			const selectedRow = page.locator(".ProseMirror table tr").nth(1);
			await selectTableCells(
				page,
				".ProseMirror table tr:nth-child(2) td:nth-child(1)",
				".ProseMirror table tr:nth-child(2) td:nth-child(2)",
			);
			await expect(mergeCellsButton).toBeEnabled();
			await mergeCellsButton.click();
			await expect(selectedRow.locator('td[colspan="2"]')).toHaveCount(1);
			await splitCellButton.click();
			await expect(selectedRow.locator('td[colspan="2"]')).toHaveCount(0);

			await page.locator(".ProseMirror table tr").nth(1).locator("td").nth(0).click();
			const headerCellCountBefore = await page.locator(".ProseMirror th").count();
			await headerCellButton.click();
			await expect(page.locator(".ProseMirror th")).toHaveCount(
				headerCellCountBefore + 1,
			);
			await headerCellButton.click();
			await expect(page.locator(".ProseMirror th")).toHaveCount(
				headerCellCountBefore,
			);

			await page.locator(".ProseMirror table tr").nth(1).locator("td").nth(0).click();
			const headerColumnCountBefore = await page.locator(".ProseMirror th").count();
			await headerColumnButton.click();
			await expect(page.locator(".ProseMirror th")).toHaveCount(
				headerColumnCountBefore + rowCountBeforeRowEdits - 1,
			);
			await headerColumnButton.click();
			await expect(page.locator(".ProseMirror th")).toHaveCount(
				headerColumnCountBefore,
			);

			await page.locator(".ProseMirror table tr").nth(1).locator("td").nth(0).click();
			const headerRowCountBefore = await page.locator(".ProseMirror th").count();
			await headerRowButton.click();
			const headerRowCountAfterFirstToggle = await page
				.locator(".ProseMirror th")
				.count();
			expect(headerRowCountAfterFirstToggle).not.toBe(headerRowCountBefore);
			await headerRowButton.click();
			await expect(page.locator(".ProseMirror th")).toHaveCount(
				headerRowCountBefore,
			);

			await page.getByRole("button", { name: "Save content" }).click();
			await expect(page.locator(".message--toast")).toContainText("Content saved.");

			await page.reload({ waitUntil: "domcontentloaded" });
			await expect(page.locator(".ProseMirror table")).toBeVisible();
			await expect(page.locator(".ProseMirror")).toContainText("Name");
			await expect(page.locator(".ProseMirror")).toContainText("Bob");
			await page.getByRole("button", { name: "Markdown" }).click();
			const savedMarkdown = await page.locator("#page_content").inputValue();
			expect(savedMarkdown).toMatch(/\|\s*Name\s*\|/);
			expect(savedMarkdown).toMatch(/\|\s*Bob\s*\|/);
			expect(savedMarkdown).toContain("Reviewing");

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("round-trips markdown tables between source and rich editor", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "table-source-user";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");
			const contentId = await createContent(harness, {
				pageType: "page",
				title: "Table source page",
				slug: "table-source-page",
				pageContent: "Initial content",
				creatorSub: subject,
			});
			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				{ waitUntil: "domcontentloaded" },
			);

			await page.getByRole("button", { name: "Markdown" }).click();
			await page.locator("#page_content").fill(
				[
					"| Name | Score |",
					"| --- | ---: |",
					"| Alice | 10 |",
					"| Bob | 7 |",
				].join("\n"),
			);

			await expect(page.locator(".ProseMirror table")).toBeVisible();
			await expect(page.locator(".ProseMirror th")).toContainText([
				"Name",
				"Score",
			]);
			await page.locator(".ProseMirror table tr").nth(1).locator("td").nth(0).click();
			await expect(page.locator("[data-table-controls]")).toBeVisible();
			await placeCaretInEditor(
				page,
				".ProseMirror table tr:nth-child(2) td:nth-child(1)",
				"Alice",
			);
			await page.keyboard.type(" Cooper");

			const updatedMarkdown = await page.locator("#page_content").inputValue();
			expect(updatedMarkdown).toMatch(/\|\s*Name\s*\|\s*Score\s*\|/);
			expect(updatedMarkdown).toMatch(/\|\s*Alice Cooper\s*\|\s*10\s*\|/);
			expect(updatedMarkdown).toMatch(/\|\s*Bob\s*\|\s*7\s*\|/);

			await page.getByRole("button", { name: "Save content" }).click();
			await expect(page.locator(".message--toast")).toContainText("Content saved.");

			await page.reload({ waitUntil: "domcontentloaded" });
			await expect(page.locator(".ProseMirror table")).toBeVisible();
			await expect(page.locator(".ProseMirror table tr").nth(1)).toContainText(
				"Alice Cooper",
			);
			await page.getByRole("button", { name: "Markdown" }).click();
			const reloadedMarkdown = await page.locator("#page_content").inputValue();
			expect(reloadedMarkdown).toMatch(/\|\s*Name\s*\|\s*Score\s*\|/);
			expect(reloadedMarkdown).toMatch(/\|\s*Alice Cooper\s*\|\s*10\s*\|/);
			expect(reloadedMarkdown).toMatch(/\|\s*Bob\s*\|\s*7\s*\|/);

			await context.close();
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

	test("inserts an image inline at the current cursor position", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "inline-image-insert";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");
			const asset = {
				originalFilename: "inline-image.png",
				storageBasename: "inline-image.png",
				thumbnailFilename: "inline-image_thumb.png",
			};
			await createAssetWithThumbnail(harness, asset);

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/new`,
				{ waitUntil: "domcontentloaded" },
			);

			await page.locator(".ProseMirror").click();
			await page.keyboard.type("Alpha omega");
			await placeCaretInEditor(
				page,
				"#editor .ProseMirror > p:first-of-type",
				"Alpha ",
			);

			await page.getByRole("button", { name: "Image" }).click();
			const modal = page.getByRole("dialog", { name: "Insert image" });
			await expect(modal).toBeVisible();
			await modal
				.locator(".asset-card", { hasText: asset.originalFilename })
				.click();
			await modal.getByLabel("Alt text").fill("Inline image");
			await modal.getByRole("button", { name: "Insert image" }).click();
			await expect(modal).toBeHidden();

			const paragraphs = page.locator("#editor .ProseMirror > p");
			await expect(paragraphs).toHaveCount(1);
			await expect(
				page.locator(
					`#editor .ProseMirror p a[href="/media/images/${asset.storageBasename}"] img[src="/media/images/${asset.thumbnailFilename}"]`,
				),
			).toBeVisible();
			await expect(paragraphs.first()).toContainText("Alpha omega");

			await page.getByRole("button", { name: "Markdown" }).click();
			const markdown = await page.locator("#page_content").inputValue();
			expect(markdown).toContain("Alpha ");
			expect(markdown).toContain("omega");
			expect(markdown).toContain(asset.thumbnailFilename);
			expect(markdown).not.toContain("\n\n");

			await context.close();
		} finally {
			await cleanupHarness(harness);
		}
	});

	test("inserts multiple selected images and clears selection when switching to an external URL", async ({
		browser,
	}) => {
		const harness = await setupHarness();

		try {
			const subject = "batch-image-insert";
			const userId = await createUser(harness, subject);
			await addMembership(harness, userId, "owner");
			const contentId = await createContent(harness, {
				pageType: "page",
				title: "Batch image edit",
				slug: "batch-image-edit",
				pageContent: "Initial body",
				creatorSub: subject,
			});
			const firstAsset = {
				originalFilename: "batch-image-alpha.png",
				storageBasename: "batch-image-alpha.png",
				thumbnailFilename: "batch-image-alpha_thumb.png",
			};
			const secondAsset = {
				originalFilename: "batch-image-beta.png",
				storageBasename: "batch-image-beta.png",
				thumbnailFilename: "batch-image-beta_thumb.png",
			};
			await createAssetWithThumbnail(harness, firstAsset);
			await createAssetWithThumbnail(harness, secondAsset);

			const { context, page } = await createAuthenticatedPage(
				browser,
				harness,
				subject,
			);

			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				{ waitUntil: "domcontentloaded" },
			);

			await page.getByRole("button", { name: "Image" }).click();
			const modal = page.getByRole("dialog", { name: "Insert image" });
			await expect(modal).toBeVisible();

			const alphaCard = modal.locator(".asset-card", {
				hasText: firstAsset.originalFilename,
			});
			const betaCard = modal.locator(".asset-card", {
				hasText: secondAsset.originalFilename,
			});
			const alphaResultCard = modal.locator(
				"[data-asset-results] .asset-card",
				{
					hasText: firstAsset.originalFilename,
				},
			);
			const betaResultCard = modal.locator(
				"[data-asset-results] .asset-card",
				{
					hasText: secondAsset.originalFilename,
				},
			);
			const altInput = modal.getByLabel("Alt text");
			const searchInput = modal.getByPlaceholder("Search by filename");
			const externalInput = modal.getByLabel("External image URL");

			await alphaCard.click();
			await expect(altInput).toHaveValue("batch image alpha");
			await expect
				.poll(() =>
					alphaCard.evaluate(
						(element) => getComputedStyle(element).backgroundColor,
					),
				)
				.toMatch(/62\.8239|76\.7419/);
			await betaCard.click();
			await modal.getByRole("heading", { name: "Insert image" }).hover();
			await expect(modal.getByText("2 images selected.")).toBeVisible();
			await expect
				.poll(() =>
					alphaCard.evaluate(
						(element) => getComputedStyle(element).backgroundColor,
					),
				)
				.toMatch(/62\.8239|76\.7419/);
			await expect
				.poll(() =>
					betaCard.evaluate(
						(element) => getComputedStyle(element).backgroundColor,
					),
				)
				.toMatch(/62\.8239|76\.7419/);
			await expect(altInput).toBeDisabled();
			await expect(altInput).toHaveValue("");
			await expect(
				modal.getByRole("button", { name: "Insert 2 images" }),
			).toBeEnabled();

			await searchInput.fill("batch-image");
			await expect(alphaResultCard).toBeVisible();
			await expect(betaResultCard).toBeVisible();
			await expect(alphaResultCard).toHaveAttribute("aria-pressed", "true");
			await expect(betaResultCard).toHaveAttribute("aria-pressed", "true");

			await alphaResultCard.click();
			await modal.getByRole("heading", { name: "Insert image" }).hover();
			await expect(modal.getByText("1 image selected.")).toBeVisible();
			await expect(altInput).toBeEnabled();
			await expect(altInput).toHaveValue("batch image beta");
			await expect(alphaResultCard).toHaveAttribute("aria-pressed", "false");
			await expect(betaResultCard).toHaveAttribute("aria-pressed", "true");
			await expect
				.poll(() =>
					alphaResultCard.evaluate(
						(element) => getComputedStyle(element).backgroundColor,
					),
				)
				.toMatch(/98\.1434|rgb\(255, 255, 255\)/);
			await expect
				.poll(() =>
					betaResultCard.evaluate(
						(element) => getComputedStyle(element).backgroundColor,
					),
				)
				.toMatch(/62\.8239|76\.7419/);

			await altInput.fill("Shared alt text");
			await alphaResultCard.click();
			await modal.getByRole("heading", { name: "Insert image" }).hover();
			await expect(modal.getByText("2 images selected.")).toBeVisible();
			await expect
				.poll(() =>
					alphaResultCard.evaluate(
						(element) => getComputedStyle(element).backgroundColor,
					),
				)
				.toMatch(/62\.8239|76\.7419/);
			await expect
				.poll(() =>
					betaResultCard.evaluate(
						(element) => getComputedStyle(element).backgroundColor,
					),
				)
				.toMatch(/62\.8239|76\.7419/);
			await expect(altInput).toBeDisabled();
			await expect(altInput).toHaveValue("Shared alt text");

			await alphaResultCard.click();
			await expect(modal.getByText("1 image selected.")).toBeVisible();
			await expect(altInput).toBeEnabled();
			await expect(altInput).toHaveValue("Shared alt text");

			await externalInput.fill("https://example.com/remote-image.png");
			await expect(
				modal.getByText("External image URL ready to insert."),
			).toBeVisible();
			await expect(alphaResultCard).toHaveAttribute("aria-pressed", "false");
			await expect(betaResultCard).toHaveAttribute("aria-pressed", "false");
			await expect(altInput).toHaveValue("Shared alt text");
			await expect(
				modal.getByRole("button", { name: "Insert image" }),
			).toBeEnabled();

			await betaResultCard.click();
			await expect(externalInput).toHaveValue("");
			await expect(modal.getByText("1 image selected.")).toBeVisible();
			await expect(altInput).toHaveValue("Shared alt text");
			await expect(betaResultCard).toHaveAttribute("aria-pressed", "true");

			await alphaResultCard.click();
			await expect(modal.getByText("2 images selected.")).toBeVisible();
			await modal.getByRole("button", { name: "Insert 2 images" }).click();
			await expect(modal).toBeHidden();

			const insertedImageLinks = await page
				.locator("#editor .ProseMirror a")
				.evaluateAll((elements) =>
					elements.map((element) => element.getAttribute("href") ?? ""),
				);
			expect(insertedImageLinks).toEqual([
				`/media/images/${secondAsset.storageBasename}`,
				`/media/images/${firstAsset.storageBasename}`,
			]);

			await page.getByRole("button", { name: "Markdown" }).click();
			const markdown = await page.locator("#page_content").inputValue();
			expect(markdown).toContain(secondAsset.thumbnailFilename);
			expect(markdown).toContain(firstAsset.thumbnailFilename);
			expect(markdown.indexOf(secondAsset.thumbnailFilename)).toBeLessThan(
				markdown.indexOf(firstAsset.thumbnailFilename),
			);

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
			const consoleErrors: string[] = [];
			page.on("console", (message) => {
				if (message.type() === "error") {
					consoleErrors.push(message.text());
				}
			});
			await page.goto(
				`https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/edit`,
				{ waitUntil: "domcontentloaded" },
			);

			const sizeSelect = page.getByRole("combobox", { name: "Size" });
			await expect(sizeSelect).toBeVisible();
			await expect(sizeSelect).toBeEnabled();
			await expect(sizeSelect).toHaveValue("normal");
			expect(
				consoleErrors.filter((message) => message.includes("contentMatchAt")),
			).toHaveLength(0);

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
