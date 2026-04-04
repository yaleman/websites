import { Editor } from "@tiptap/core";
import Image from "@tiptap/extension-image";
import Link from "@tiptap/extension-link";
import { Markdown } from "@tiptap/markdown";
import StarterKit from "@tiptap/starter-kit";
import "./editor.css";
import type { Client } from "openapi-fetch";
import createClient from "openapi-fetch";
import { ApiPaths, type components, type paths } from "./openapi";

let OPENAPI_CLIENT: Client<paths, `${string}/${string}`>;

type AssetLibraryItem = components["schemas"]["AssetLibraryItem"];
const HEADING_LEVELS = [1, 2, 3, 4, 5, 6] as const;

const inferAltFromFilename = (filename: string) => {
	const trimmed = filename.replace(/\.[^/.]+$/, "");
	return trimmed.replace(/[-_]+/g, " ").replace(/\s+/g, " ").trim();
};

const formatAssetSize = (byteLength: number) => {
	if (byteLength < 1024) {
		return `${byteLength} B`;
	}
	if (byteLength < 1024 * 1024) {
		return `${(byteLength / 1024).toFixed(1)} KB`;
	}
	return `${(byteLength / (1024 * 1024)).toFixed(1)} MB`;
};

const formatAssetDate = (value: string) => {
	const parsed = new Date(value);
	if (Number.isNaN(parsed.getTime())) {
		return value;
	}
	return new Intl.DateTimeFormat(undefined, {
		dateStyle: "medium",
		timeStyle: "short",
	}).format(parsed);
};

const formatAssetMeta = (asset: AssetLibraryItem) => {
	const dimensions =
		asset.width && asset.height ? `${asset.width}×${asset.height}` : "size n/a";
	return `${asset.mime_type} • ${dimensions} • ${formatAssetSize(asset.byte_length)} • ${formatAssetDate(asset.created_at)}`;
};

const bindConfirmingForms = () => {
	document
		.querySelectorAll<HTMLFormElement>("form[data-confirm-message]")
		.forEach((form) => {
			form.addEventListener("submit", (event) => {
				const message = form.dataset.confirmMessage?.trim();
				if (!message) {
					return;
				}
				if (!window.confirm(message)) {
					event.preventDefault();
				}
			});
		});
};

const assetLibraryRefreshKey = (siteId: string) => `site-assets-updated:${siteId}`;

const announceAssetLibraryUpdate = () => {
	const currentPath = window.location.pathname;
	const currentMatch = currentPath.match(/^\/admin\/site\/([^/]+)\/assets$/);
	if (!currentMatch) {
		return;
	}

	let referrerPath = "";
	try {
		if (document.referrer) {
			referrerPath = new URL(document.referrer).pathname;
		}
	} catch {
		return;
	}

	const [, siteId] = currentMatch;
	if (
		referrerPath !== `/admin/site/${siteId}/assets/new` &&
		!new RegExp(`^/admin/site/${siteId}/assets/[^/]+/replace$`).test(
			referrerPath,
		)
	) {
		return;
	}

	window.localStorage.setItem(assetLibraryRefreshKey(siteId), `${Date.now()}`);
};

const normalizeSlug = (value: string) => {
	let slug = "";
	let previousDash = false;

	for (const ch of value.toLowerCase()) {
		const code = ch.charCodeAt(0);
		const isAsciiAlphaNumeric =
			(code >= 48 && code <= 57) || (code >= 97 && code <= 122);

		if (isAsciiAlphaNumeric) {
			slug += ch;
			previousDash = false;
			continue;
		}

		if (!previousDash) {
			slug += "-";
			previousDash = true;
		}
	}

	slug = slug.trim().replace(/^[-_]+|[-_]+$/g, "");
	return slug || "post";
};

const bindNewContentSlugController = () => {
	const form = document.querySelector<HTMLFormElement>(
		"form[data-new-content-slug-controller]",
	);

	if (!form) {
		return;
	}

	const titleInput = form.querySelector<HTMLInputElement>("#title");
	const slugInput = form.querySelector<HTMLInputElement>("#slug");
	const resetButton = form.querySelector<HTMLButtonElement>("[data-slug-reset]");

	if (!titleInput || !slugInput || !resetButton) {
		return;
	}

	const promptMessage =
		"Edit the slug manually? It will stop following the title until you reset it.";
	let isManualSlug = false;

	const syncSlugFromTitle = () => {
		if (isManualSlug) {
			return;
		}
		const title = titleInput.value.trim();
		slugInput.value = title ? normalizeSlug(title) : "";
	};

	const setManualMode = (manual: boolean) => {
		isManualSlug = manual;
		slugInput.readOnly = !manual;
	};

	const confirmManualEdit = () => {
		if (isManualSlug) {
			return true;
		}
		if (!window.confirm(promptMessage)) {
			return false;
		}
		setManualMode(true);
		return true;
	};

	const unlockSlugFromPointer = (event: PointerEvent) => {
		if (isManualSlug) {
			return;
		}
		event.preventDefault();
		if (!confirmManualEdit()) {
			return;
		}
		slugInput.focus();
		slugInput.select();
	};

	const unlockSlugFromKeyboard = (event: KeyboardEvent) => {
		if (isManualSlug) {
			return;
		}

		const key = event.key.toLowerCase();
		const isEditingKey =
			event.key.length === 1 ||
			event.key === "Backspace" ||
			event.key === "Delete" ||
			((event.ctrlKey || event.metaKey) && (key === "v" || key === "x"));

		if (!isEditingKey) {
			return;
		}

		event.preventDefault();
		if (!confirmManualEdit()) {
			return;
		}
		slugInput.focus();
		slugInput.select();
	};

	resetButton.addEventListener("click", (event) => {
		event.preventDefault();
		setManualMode(false);
		syncSlugFromTitle();
		slugInput.focus();
	});

	titleInput.addEventListener("input", syncSlugFromTitle);
	titleInput.addEventListener("change", syncSlugFromTitle);

	slugInput.addEventListener("pointerdown", unlockSlugFromPointer);
	slugInput.addEventListener("keydown", unlockSlugFromKeyboard);

	form.addEventListener("submit", () => {
		syncSlugFromTitle();
	});

	setManualMode(false);
	syncSlugFromTitle();
};

const createAssetCard = (asset: AssetLibraryItem, selected: boolean) => {
	const button = document.createElement("button");
	button.type = "button";
	button.className =
		"asset-card group grid gap-2 rounded-md border border-slate-200 bg-white p-2 text-left shadow-sm transition hover:border-slate-300 hover:bg-slate-50 focus:outline-none focus:ring-2 focus:ring-slate-200";
	button.dataset.assetId = asset.id;
	button.dataset.assetPayload = JSON.stringify(asset);
	button.dataset.assetCard = "true";
	button.classList.toggle("is-selected", selected);
	button.setAttribute("aria-pressed", selected ? "true" : "false");

	const thumb = document.createElement("div");
	thumb.className = "aspect-[4/3] overflow-hidden rounded-md bg-slate-100";
	const image = document.createElement("img");
	image.src = asset.thumbnail_url ?? asset.original_url;
	image.alt = "";
	image.loading = "lazy";
	thumb.appendChild(image);

	const name = document.createElement("div");
	name.className = "truncate text-sm font-medium text-slate-900";
	name.textContent = asset.original_filename;

	const meta = document.createElement("div");
	meta.className = "text-xs text-slate-500";
	meta.textContent = formatAssetMeta(asset);

	button.appendChild(thumb);
	button.appendChild(name);
	button.appendChild(meta);
	return button;
};

const renderAssetGrid = (
	container: HTMLElement,
	assets: AssetLibraryItem[],
	message: string,
	selectedAssetIds: Set<string>,
) => {
	container.innerHTML = "";
	if (assets.length === 0) {
		const empty = document.createElement("div");
		empty.className = "text-xs text-slate-500";
		empty.textContent = message;
		container.appendChild(empty);
		return;
	}
	for (const asset of assets) {
		container.appendChild(
			createAssetCard(asset, selectedAssetIds.has(asset.id)),
		);
	}
};

const createAssetModal = (editor: Editor) => {
	const modal = document.querySelector<HTMLElement>("[data-asset-modal]");
	const configRoot = document.querySelector<HTMLElement>(
		"[data-editor-config]",
	);
	const siteId = configRoot?.dataset.siteId;

	if (!modal || !siteId) {
		return { open: () => {} };
	}

	const closeButtons = modal.querySelectorAll<HTMLElement>(
		"[data-asset-modal-close]",
	);
	const searchInput = modal.querySelector<HTMLInputElement>(
		"[data-asset-search]",
	);
	const typeSelect =
		modal.querySelector<HTMLSelectElement>("[data-asset-type]");
	const sortBySelect = modal.querySelector<HTMLSelectElement>(
		"[data-asset-sort-by]",
	);
	const sortDirSelect = modal.querySelector<HTMLSelectElement>(
		"[data-asset-sort-dir]",
	);
	const recentSection = modal.querySelector<HTMLElement>(
		"[data-asset-recent-section]",
	);
	const resultsSection = modal.querySelector<HTMLElement>(
		"[data-asset-results-section]",
	);
	const recentGrid = modal.querySelector<HTMLElement>("[data-asset-recent]");
	const resultsGrid = modal.querySelector<HTMLElement>("[data-asset-results]");
	const altInput = modal.querySelector<HTMLInputElement>("[data-asset-alt]");
	const externalInput = modal.querySelector<HTMLInputElement>(
		"[data-asset-external]",
	);
	const selectionSummary = modal.querySelector<HTMLElement>(
		"[data-asset-selection-summary]",
	);
	const selectionHint = modal.querySelector<HTMLElement>(
		"[data-asset-selection-hint]",
	);
	const altHelp = modal.querySelector<HTMLElement>("[data-asset-alt-help]");
	const insertButton = modal.querySelector<HTMLButtonElement>(
		"[data-asset-insert]",
	);

	let selectedAssets: AssetLibraryItem[] = [];
	let sharedAltText = "";
	let hasExplicitSharedAlt = false;
	let searchTimeout: number | null = null;
	let isModalOpen = false;
	let refreshPromise: Promise<void> | null = null;

	const selectedAssetIds = () =>
		new Set(selectedAssets.map((asset) => asset.id));

	const syncCardSelection = () => {
		const ids = selectedAssetIds();
		modal.querySelectorAll<HTMLElement>("[data-asset-card]").forEach((card) => {
			const selected = ids.has(card.getAttribute("data-asset-id") ?? "");
			card.classList.toggle("is-selected", selected);
			card.setAttribute("aria-pressed", selected ? "true" : "false");
		});
	};

	const setModalOpen = (open: boolean) => {
		isModalOpen = open;
		if (open) {
			modal.removeAttribute("hidden");
			modal.setAttribute("aria-hidden", "false");
			document.body?.classList.add("modal-open");
			searchInput?.focus();
		} else {
			modal.setAttribute("hidden", "");
			modal.setAttribute("aria-hidden", "true");
			document.body?.classList.remove("modal-open");
		}
	};

	const updateSelectionUi = () => {
		const externalUrl = externalInput?.value.trim() ?? "";
		const selectionCount = selectedAssets.length;
		const singleSelectedAsset =
			selectionCount === 1 ? selectedAssets[0] : null;
		const displayedAltText = hasExplicitSharedAlt
			? sharedAltText
			: externalUrl
				? ""
				: singleSelectedAsset
					? inferAltFromFilename(singleSelectedAsset.original_filename)
					: "";

		if (selectionSummary) {
			if (externalUrl) {
				selectionSummary.textContent = "External image URL ready to insert.";
			} else if (selectionCount === 0) {
				selectionSummary.textContent = "No images selected.";
			} else if (selectionCount === 1) {
				selectionSummary.textContent = "1 image selected.";
			} else {
				selectionSummary.textContent = `${selectionCount} images selected.`;
			}
		}

		if (selectionHint) {
			if (externalUrl) {
				selectionHint.textContent =
					"Selecting a library image clears the external URL.";
			} else if (selectionCount > 1) {
				selectionHint.textContent =
					"Batch insert uses alt text inferred from each filename.";
			} else {
				selectionHint.textContent =
					"Shared alt text applies to a single selected image or an external image URL.";
			}
		}

		if (altInput) {
			const usesSharedAlt = Boolean(externalUrl) || selectionCount <= 1;
			altInput.disabled = !usesSharedAlt;
			altInput.value = usesSharedAlt
				? displayedAltText
				: hasExplicitSharedAlt
					? sharedAltText
					: "";
		}

		if (altHelp) {
			altHelp.textContent =
				externalUrl || selectionCount <= 1
					? "Used for a single selected image or an external image URL."
					: "Ignored for batch insert. Each image uses alt text inferred from its filename.";
		}

		if (!insertButton) {
			return;
		}
		insertButton.disabled = selectionCount === 0 && !externalUrl;
		insertButton.textContent =
			externalUrl || selectionCount <= 1
				? "Insert image"
				: `Insert ${selectionCount} images`;
	};

	const clearSelectedAssets = () => {
		selectedAssets = [];
		syncCardSelection();
		updateSelectionUi();
	};

	const toggleSelectedAsset = (asset: AssetLibraryItem) => {
		const existingIndex = selectedAssets.findIndex(
			(selectedAsset) => selectedAsset.id === asset.id,
		);
		if (existingIndex >= 0) {
			selectedAssets.splice(existingIndex, 1);
			syncCardSelection();
			updateSelectionUi();
			return;
		}

		if (externalInput?.value.trim()) {
			externalInput.value = "";
		}
		selectedAssets = [...selectedAssets, asset];
		syncCardSelection();
		updateSelectionUi();
	};

	const setSectionVisibility = (showResults: boolean) => {
		if (recentSection) {
			recentSection.toggleAttribute("hidden", showResults);
		}
		if (resultsSection) {
			resultsSection.toggleAttribute("hidden", !showResults);
		}
	};

	const fetchAssets = async (options: {
		query?: string;
		limit: number;
		type: string;
		sortBy: string;
		sortDir: string;
	}) => {
		const { data, error } = await OPENAPI_CLIENT.GET(
			ApiPaths.api_site_assets_library,
			{
				cache: "no-store",
				params: {
					path: { site_id: siteId },
					query: {
						q: options.query || "",
						type: options.type,
						limit: options.limit,
						sort_by: options.sortBy,
						sort_dir: options.sortDir,
					},
				},
			},
		);

		if (error || !data) {
			throw new Error(`Failed to fetch assets. ${error}`);
		}
		const payload = data as components["schemas"]["AssetLibraryResponse"];
		return payload.assets ?? [];
	};

	const restoreSelection = (assets: AssetLibraryItem[]) => {
		if (selectedAssets.length === 0) {
			syncCardSelection();
			updateSelectionUi();
			return;
		}
		const assetsById = new Map(assets.map((asset) => [asset.id, asset]));
		selectedAssets = selectedAssets.map(
			(asset) => assetsById.get(asset.id) ?? asset,
		);
		syncCardSelection();
		updateSelectionUi();
	};

	const refreshVisibleAssets = async () => {
		if (refreshPromise) {
			await refreshPromise;
			return;
		}

		refreshPromise = (async () => {
			const query = searchInput?.value.trim() ?? "";
			const selectedType = typeSelect?.value ?? "all";
			const selectedSortBy = sortBySelect?.value ?? "uploaded";
			const selectedSortDir = sortDirSelect?.value ?? "desc";
			const showingResults = query.length > 0;

			setSectionVisibility(showingResults);

			if (showingResults) {
				if (!resultsGrid) {
					return;
				}
				renderAssetGrid(resultsGrid, [], "Searching...", selectedAssetIds());
				try {
					const assets = await fetchAssets({
						query,
						limit: 50,
						type: selectedType,
						sortBy: selectedSortBy,
						sortDir: selectedSortDir,
					});
					renderAssetGrid(
						resultsGrid,
						assets,
						"No matches found.",
						selectedAssetIds(),
					);
					restoreSelection(assets);
				} catch {
					renderAssetGrid(
						resultsGrid,
						[],
						"Unable to load search results.",
						selectedAssetIds(),
					);
					syncCardSelection();
					updateSelectionUi();
				}
				return;
			}

			if (!recentGrid) {
				return;
			}
			renderAssetGrid(
				recentGrid,
				[],
				"Loading recent images...",
				selectedAssetIds(),
			);
			try {
				const assets = await fetchAssets({
					limit: 12,
					type: selectedType,
					sortBy: selectedSortBy,
					sortDir: selectedSortDir,
				});
				renderAssetGrid(
					recentGrid,
					assets,
					"No images uploaded yet.",
					selectedAssetIds(),
				);
				restoreSelection(assets);
			} catch {
				renderAssetGrid(
					recentGrid,
					[],
					"Unable to load recent images.",
					selectedAssetIds(),
				);
				syncCardSelection();
				updateSelectionUi();
			}
		})();

		try {
			await refreshPromise;
		} finally {
			refreshPromise = null;
		}
	};

	const scheduleSearch = () => {
		if (searchTimeout) {
			window.clearTimeout(searchTimeout);
		}
		searchTimeout = window.setTimeout(() => {
			void refreshVisibleAssets();
		}, 300);
	};

	const handleAssetClick = (event: Event) => {
		const target = event.target as HTMLElement | null;
		if (!target) {
			return;
		}
		const card = target.closest<HTMLButtonElement>("[data-asset-card]");
		if (!card) {
			return;
		}
		const payload = card.dataset.assetPayload;
		if (!payload) {
			return;
		}
		const asset = JSON.parse(
			payload,
		) as AssetLibraryItem;
		toggleSelectedAsset(asset);
	};

	const buildInlineImageNode = (options: {
		src: string;
		alt?: string;
		href?: string;
	}) => {
		return {
			type: "image",
			attrs: {
				src: options.src,
				...(options.alt ? { alt: options.alt } : {}),
			},
			...(options.href
				? {
						marks: [
							{
								type: "link",
								attrs: {
									href: options.href,
								},
							},
						],
					}
				: {}),
		};
	};

	const insertSelection = () => {
		const altText = hasExplicitSharedAlt ? sharedAltText.trim() : undefined;
		const externalUrl = externalInput?.value.trim();
		if (externalUrl) {
			editor
				.chain()
				.focus()
				.insertContent([buildInlineImageNode({ src: externalUrl, alt: altText })])
				.run();
			close();
			return;
		}

		if (selectedAssets.length === 0) {
			return;
		}
		editor
			.chain()
			.focus()
			.insertContent(
				selectedAssets.map((selectedAsset) =>
					buildInlineImageNode({
						src:
							selectedAsset.has_thumbnail && selectedAsset.thumbnail_url
								? selectedAsset.thumbnail_url
								: selectedAsset.original_url,
						alt:
							selectedAssets.length === 1
								? altText
								: inferAltFromFilename(selectedAsset.original_filename),
						...(selectedAsset.has_thumbnail && selectedAsset.thumbnail_url
							? { href: selectedAsset.original_url }
							: {}),
					}),
				),
			)
			.run();
		close();
	};

	const resetModalState = () => {
		if (searchInput) {
			searchInput.value = "";
		}
		if (searchTimeout) {
			window.clearTimeout(searchTimeout);
			searchTimeout = null;
		}
		if (sortBySelect) {
			sortBySelect.value = "uploaded";
		}
		if (sortDirSelect) {
			sortDirSelect.value = "desc";
		}
		if (altInput) {
			altInput.value = "";
		}
		sharedAltText = "";
		hasExplicitSharedAlt = false;
		if (externalInput) {
			externalInput.value = "";
		}
		setSectionVisibility(false);
		clearSelectedAssets();
	};

	const open = () => {
		resetModalState();
		setModalOpen(true);
		void refreshVisibleAssets();
	};

	const close = () => {
		setModalOpen(false);
		resetModalState();
	};

	closeButtons.forEach((button) => {
		button.addEventListener("click", (event) => {
			event.preventDefault();
			close();
		});
	});

	modal.addEventListener("keydown", (event) => {
		if (event.key === "Escape") {
			close();
		}
	});

	window.addEventListener("focus", () => {
		if (!isModalOpen) {
			return;
		}
		void refreshVisibleAssets();
	});

	document.addEventListener("visibilitychange", () => {
		if (!isModalOpen || document.visibilityState !== "visible") {
			return;
		}
		void refreshVisibleAssets();
	});

	window.addEventListener("storage", (event) => {
		if (!isModalOpen || event.key !== assetLibraryRefreshKey(siteId)) {
			return;
		}
		void refreshVisibleAssets();
	});

	searchInput?.addEventListener("input", scheduleSearch);
	searchInput?.addEventListener("keydown", (event) => {
		if (event.key === "Enter") {
			event.preventDefault();
			scheduleSearch();
		}
	});
	typeSelect?.addEventListener("change", scheduleSearch);
	sortBySelect?.addEventListener("change", () => {
		void refreshVisibleAssets();
	});
	sortDirSelect?.addEventListener("change", () => {
		void refreshVisibleAssets();
	});
	externalInput?.addEventListener("input", () => {
		if (externalInput.value.trim()) {
			clearSelectedAssets();
			return;
		}
		updateSelectionUi();
	});
	externalInput?.addEventListener("keydown", (event) => {
		if (event.key === "Enter") {
			event.preventDefault();
			insertSelection();
		}
	});
	altInput?.addEventListener("input", () => {
		sharedAltText = altInput.value;
		hasExplicitSharedAlt = true;
		updateSelectionUi();
	});
	insertButton?.addEventListener("click", (event) => {
		event.preventDefault();
		insertSelection();
	});

	if (recentGrid) {
		recentGrid.addEventListener("click", handleAssetClick);
	}
	if (resultsGrid) {
		resultsGrid.addEventListener("click", handleAssetClick);
	}

	updateSelectionUi();

	return { open };
};

const bindToolbar = (
	editor: Editor,
	textarea: HTMLTextAreaElement,
	openAssetModal: () => void,
) => {
	const toolbar = document.querySelector<HTMLElement>("[data-editor-toolbar]");
	const previewContainer = document.querySelector<HTMLElement>(
		"[data-editor-preview]",
	);
	const previewBody = document.querySelector<HTMLElement>(
		"[data-editor-preview-body]",
	);
	const sourcePanel = document.querySelector<HTMLElement>(
		"[data-editor-source-panel]",
	);
	const previewButton = toolbar?.querySelector<HTMLButtonElement>(
		'button[data-command="preview"]',
	);
	const sourceButton = toolbar?.querySelector<HTMLButtonElement>(
		'button[data-command="source"]',
	);
	const sizeControl = toolbar?.querySelector<HTMLSelectElement>(
		'select[data-command="size"]',
	);
	const formattingButtons = {
		bold: toolbar?.querySelector<HTMLButtonElement>('button[data-command="bold"]'),
		italic: toolbar?.querySelector<HTMLButtonElement>(
			'button[data-command="italic"]',
		),
		code: toolbar?.querySelector<HTMLButtonElement>('button[data-command="code"]'),
		link: toolbar?.querySelector<HTMLButtonElement>('button[data-command="link"]'),
		ul: toolbar?.querySelector<HTMLButtonElement>('button[data-command="ul"]'),
		ol: toolbar?.querySelector<HTMLButtonElement>('button[data-command="ol"]'),
		quote: toolbar?.querySelector<HTMLButtonElement>('button[data-command="quote"]'),
	};
	const formattingCommandNames = new Set([
		"bold",
		"italic",
		"code",
		"link",
		"image",
		"ul",
		"ol",
		"quote",
	]);

	const setButtonState = (
		button: HTMLButtonElement | null | undefined,
		active: boolean,
	) => {
		if (!button) {
			return;
		}
		button.classList.toggle("is-active", active);
		button.setAttribute("aria-pressed", active ? "true" : "false");
	};

	const setButtonEnabled = (
		button: HTMLButtonElement | null | undefined,
		enabled: boolean,
	) => {
		if (!button) {
			return;
		}
		button.disabled = !enabled;
	};

	const activeSizeValue = () => {
		for (const level of HEADING_LEVELS) {
			if (editor.isActive("heading", { level })) {
				return `h${level}`;
			}
		}

		return "normal";
	};

	const syncSizeControl = () => {
		if (!sizeControl) {
			return;
		}

		sizeControl.value = activeSizeValue();
		sizeControl.disabled = !editor.isEditable;
	};

	const syncToolbarState = () => {
		setButtonState(formattingButtons.bold, editor.isActive("bold"));
		setButtonState(formattingButtons.italic, editor.isActive("italic"));
		setButtonState(formattingButtons.code, editor.isActive("code"));
		setButtonState(formattingButtons.link, editor.isActive("link"));
		setButtonState(formattingButtons.ul, editor.isActive("bulletList"));
		setButtonState(formattingButtons.ol, editor.isActive("orderedList"));
		setButtonState(formattingButtons.quote, editor.isActive("blockquote"));
		syncSizeControl();

		setButtonEnabled(
			formattingButtons.bold,
			editor.can().chain().focus().toggleBold().run(),
		);
		setButtonEnabled(
			formattingButtons.italic,
			editor.can().chain().focus().toggleItalic().run(),
		);
		setButtonEnabled(
			formattingButtons.code,
			editor.can().chain().focus().toggleCode().run(),
		);
		setButtonEnabled(
			formattingButtons.link,
			editor.can().chain().focus().setLink({ href: "https://example.com" }).run() ||
				editor.isActive("link"),
		);
		setButtonEnabled(
			formattingButtons.ul,
			editor.can().chain().focus().toggleBulletList().run(),
		);
		setButtonEnabled(
			formattingButtons.ol,
			editor.can().chain().focus().toggleOrderedList().run(),
		);
		setButtonEnabled(
			formattingButtons.quote,
			editor.can().chain().focus().toggleBlockquote().run(),
		);
	};

	const updatePreview = () => {
		if (!previewBody) {
			return;
		}
		const previewVisible = !previewContainer?.hasAttribute("hidden");
		const sourceVisible = !sourcePanel?.hasAttribute("hidden");
		if (!previewVisible && !sourceVisible) {
			return;
		}
		const previewContent = editor.view.dom.cloneNode(true);
		if (previewContent instanceof HTMLElement) {
			previewContent.removeAttribute("contenteditable");
			previewContent.classList.remove("ProseMirror");
			previewContent
				.querySelectorAll<HTMLElement>(".ProseMirror")
				.forEach((element) => {
					element.classList.remove("ProseMirror");
				});
			previewContent
				.querySelectorAll<HTMLElement>("[contenteditable]")
				.forEach((element) => {
					element.removeAttribute("contenteditable");
				});
		}
		previewBody.replaceChildren(previewContent);
	};

	const setPreviewVisible = (visible: boolean) => {
		if (!previewContainer) {
			return;
		}
		if (visible) {
			previewContainer.removeAttribute("hidden");
		} else {
			previewContainer.setAttribute("hidden", "");
		}
		if (previewButton) {
			previewButton.classList.toggle("is-active", visible);
			previewButton.setAttribute("aria-pressed", visible ? "true" : "false");
		}
	};

	const setSourceVisible = (visible: boolean) => {
		if (!sourcePanel) {
			return;
		}
		if (visible) {
			sourcePanel.removeAttribute("hidden");
			textarea.focus();
		} else {
			sourcePanel.setAttribute("hidden", "");
		}
		if (sourceButton) {
			sourceButton.classList.toggle("is-active", visible);
			sourceButton.setAttribute("aria-pressed", visible ? "true" : "false");
		}
	};

	const togglePreview = () => {
		if (!previewContainer) {
			return;
		}
		const willShow = previewContainer.hasAttribute("hidden");
		setPreviewVisible(willShow);
		if (willShow) {
			updatePreview();
		}
	};

	const toggleSource = () => {
		if (!sourcePanel) {
			return;
		}
		const willShow = sourcePanel.hasAttribute("hidden");
		setSourceVisible(willShow);
		if (willShow) {
			updatePreview();
		}
	};

	if (!toolbar) {
		return { updatePreview, setSourceVisible, syncToolbarState };
	}

	const handleCommand = (commandName: string) => {
		switch (commandName) {
			case "bold":
				editor.chain().focus().toggleBold().run();
				return;
			case "italic":
				editor.chain().focus().toggleItalic().run();
				return;
			case "code":
				editor.chain().focus().toggleCode().run();
				return;
			case "link": {
				const href = window.prompt("Link URL");
				if (!href) {
					editor.chain().focus().unsetLink().run();
					return;
				}
				editor.chain().focus().setLink({ href }).run();
				return;
			}
			case "image": {
				openAssetModal();
				return;
			}
			case "ul":
				editor.chain().focus().toggleBulletList().run();
				return;
			case "ol":
				editor.chain().focus().toggleOrderedList().run();
				return;
			case "quote":
				editor.chain().focus().toggleBlockquote().run();
				return;
			case "preview":
				togglePreview();
				return;
			case "source":
				toggleSource();
				return;
			default:
				return;
		}
	};

	const getToolbarButton = (target: EventTarget | null) => {
		if (!(target instanceof HTMLElement)) {
			return null;
		}

		return target.closest<HTMLButtonElement>("button[data-command]");
	};

	toolbar.addEventListener("pointerdown", (event) => {
		const button = getToolbarButton(event.target);
		if (!button) {
			return;
		}

		const commandName = button.getAttribute("data-command");
		if (!commandName || !formattingCommandNames.has(commandName)) {
			return;
		}

		event.preventDefault();
	});

	toolbar.addEventListener("click", (event) => {
		const button = getToolbarButton(event.target);
		if (!button) {
			return;
		}

		const commandName = button.getAttribute("data-command");
		if (!commandName) {
			return;
		}

		event.preventDefault();
		handleCommand(commandName);
		syncToolbarState();
	});

	sizeControl?.addEventListener("change", (event) => {
		event.preventDefault();
		const sizeValue = sizeControl.value;
		if (sizeValue === "normal") {
			editor.chain().focus().setParagraph().run();
			syncToolbarState();
			return;
		}

		const parsedLevel = Number.parseInt(sizeValue.replace(/^h/, ""), 10);
		const level = HEADING_LEVELS.find(
			(headingLevel) => headingLevel === parsedLevel,
		);
		if (!level) {
			syncToolbarState();
			return;
		}

		editor.chain().focus().toggleHeading({ level }).run();
		syncToolbarState();
	});

	editor.on("transaction", syncToolbarState);
	editor.on("selectionUpdate", syncToolbarState);
	editor.on("update", syncToolbarState);
	syncToolbarState();

	return { updatePreview, setSourceVisible, syncToolbarState };
};

const initTransientMessages = () => {
	const messages = document.querySelectorAll<HTMLElement>(
		"[data-auto-dismiss-ms]",
	);

	for (const message of messages) {
		const dismissMs = Number.parseInt(message.dataset.autoDismissMs ?? "", 10);
		if (!Number.isFinite(dismissMs) || dismissMs <= 0) {
			continue;
		}

		window.setTimeout(() => {
			message.setAttribute("hidden", "");
			const queryParam = message.dataset.clearQueryParam;
			if (queryParam) {
				const url = new URL(window.location.href);
				if (url.searchParams.has(queryParam)) {
					url.searchParams.delete(queryParam);
					const nextUrl = `${url.pathname}${url.search}${url.hash}`;
					window.history.replaceState({}, "", nextUrl);
				}
			}
		}, dismissMs);
	}
};

const initTagEditor = () => {
	const form = document.querySelector<HTMLFormElement>("form.editor-form");
	const input = document.querySelector<HTMLInputElement>("[data-tag-input]");
	const hiddenInput =
		document.querySelector<HTMLInputElement>("[data-tag-list]");
	const chipContainer = document.querySelector<HTMLElement>("[data-tag-chips]");
	const datalist = document.getElementById("tag-suggestions");

	if (!form || !input || !hiddenInput || !chipContainer) {
		return;
	}

	const existingTagMap = new Map<string, string>();
	for (const option of datalist?.querySelectorAll("option") ?? []) {
		const value = option.getAttribute("value")?.trim();
		if (!value) {
			continue;
		}
		existingTagMap.set(value.toLowerCase(), value);
	}

	const selectedTags = Array.from(
		chipContainer.querySelectorAll<HTMLElement>("[data-tag-chip]"),
	)
		.map((chip) => chip.dataset.tagChip?.trim() ?? "")
		.filter((tag) => tag.length > 0);

	const normalizeTag = (value: string) => {
		const trimmed = value.trim().replace(/\s+/g, " ");
		if (!trimmed) {
			return "";
		}
		return existingTagMap.get(trimmed.toLowerCase()) ?? trimmed;
	};

	const syncSelectedTags = () => {
		hiddenInput.value = selectedTags.join("\n");
	};

	const renderTags = () => {
		chipContainer.innerHTML = "";
		for (const tag of selectedTags) {
			const chip = document.createElement("button");
			chip.type = "button";
			chip.className =
				"inline-flex items-center gap-2 rounded-md border border-slate-300 bg-white px-2.5 py-1 text-sm font-medium text-slate-700 shadow-sm";
			chip.dataset.tagChip = tag;

			const label = document.createElement("span");
			label.textContent = tag;
			chip.appendChild(label);

			const remove = document.createElement("span");
			remove.className = "text-xs font-semibold text-slate-400";
			remove.setAttribute("aria-hidden", "true");
			remove.textContent = "x";
			chip.appendChild(remove);

			chip.addEventListener("click", () => {
				const index = selectedTags.indexOf(tag);
				if (index >= 0) {
					selectedTags.splice(index, 1);
					syncSelectedTags();
					renderTags();
				}
			});

			chipContainer.appendChild(chip);
		}
	};

	const addTag = (raw: string) => {
		const tag = normalizeTag(raw);
		if (!tag) {
			return;
		}
		if (
			selectedTags.some(
				(existing) => existing.toLowerCase() === tag.toLowerCase(),
			)
		) {
			input.value = "";
			return;
		}
		selectedTags.push(tag);
		syncSelectedTags();
		renderTags();
		input.value = "";
	};

	input.addEventListener("keydown", (event) => {
		if (event.key === "Enter" || event.key === ",") {
			event.preventDefault();
			addTag(input.value.replace(/,+$/, ""));
			return;
		}
		if (event.key === "Backspace" && !input.value && selectedTags.length > 0) {
			selectedTags.pop();
			syncSelectedTags();
			renderTags();
		}
	});

	input.addEventListener("blur", () => {
		if (input.value.trim()) {
			addTag(input.value);
		}
	});

	form.addEventListener("submit", () => {
		if (input.value.trim()) {
			addTag(input.value);
		}
		syncSelectedTags();
	});

	syncSelectedTags();
	renderTags();
};

const initMembershipCreateForm = () => {
	const form = document.querySelector<HTMLFormElement>(
		"[data-membership-create]",
	);
	const autocomplete = form?.querySelector<HTMLElement>(
		"[data-membership-autocomplete]",
	);
	const queryInput = form?.querySelector<HTMLInputElement>(
		"[data-membership-user-query]",
	);
	const userIdInput = form?.querySelector<HTMLInputElement>(
		"[data-membership-user-id]",
	);
	const optionsContainer = form?.querySelector<HTMLElement>(
		"[data-membership-user-options]",
	);
	const emptyState = form?.querySelector<HTMLElement>(
		"[data-membership-empty]",
	);

	if (
		!form ||
		!autocomplete ||
		!queryInput ||
		!userIdInput ||
		!optionsContainer ||
		!emptyState
	) {
		return;
	}

	const candidates = Array.from(
		optionsContainer.querySelectorAll<HTMLButtonElement>(
			"[data-membership-option]",
		),
	)
		.map((option) => ({
			element: option,
			value: option.dataset.searchValue?.trim() ?? "",
			userId: option.dataset.userId ?? "",
			subject: option.dataset.userSubject?.trim() ?? "",
			email: option.dataset.userEmail?.trim() ?? "",
		}))
		.filter(
			(candidate) => candidate.value.length > 0 && candidate.userId.length > 0,
		);

	const normalize = (value: string) => value.trim().toLowerCase();

	const resolveCandidate = (rawValue: string) => {
		const normalized = normalize(rawValue);
		if (!normalized) {
			return null;
		}
		return (
			candidates.find(
				(candidate) => normalize(candidate.value) === normalized,
			) ??
			candidates.find(
				(candidate) => normalize(candidate.subject) === normalized,
			) ??
			candidates.find(
				(candidate) => normalize(candidate.email) === normalized,
			) ??
			null
		);
	};

	const filterCandidates = (rawValue: string) => {
		const normalized = normalize(rawValue);
		if (!normalized) {
			return candidates.slice(0, 8);
		}
		return candidates
			.filter((candidate) => {
				const haystacks = [candidate.value, candidate.subject, candidate.email]
					.map((value) => normalize(value))
					.filter((value) => value.length > 0);
				return haystacks.some((value) => value.includes(normalized));
			})
			.slice(0, 8);
	};

	const renderMatches = (matches: typeof candidates) => {
		for (const candidate of candidates) {
			candidate.element.hidden = !matches.includes(candidate);
		}
		emptyState.hidden = matches.length > 0 || !queryInput.value.trim();
		const shouldShow =
			document.activeElement === queryInput &&
			(matches.length > 0 || !emptyState.hidden);
		optionsContainer.hidden = !shouldShow;
		queryInput.setAttribute("aria-expanded", shouldShow ? "true" : "false");
	};

	const applyCandidate = (candidate: (typeof candidates)[number]) => {
		queryInput.value = candidate.value;
		userIdInput.value = candidate.userId;
		queryInput.setCustomValidity("");
		optionsContainer.hidden = true;
		queryInput.setAttribute("aria-expanded", "false");
	};

	const syncSelection = () => {
		const match = resolveCandidate(queryInput.value);
		userIdInput.value = match?.userId ?? "";
		if (queryInput.value.trim() && !match) {
			renderMatches(filterCandidates(queryInput.value));
		}
		return match;
	};

	queryInput.addEventListener("input", () => {
		userIdInput.value = "";
		queryInput.setCustomValidity("");
		const match = resolveCandidate(queryInput.value);
		if (match) {
			userIdInput.value = match.userId;
		}
		renderMatches(filterCandidates(queryInput.value));
	});

	queryInput.addEventListener("change", () => {
		syncSelection();
	});

	queryInput.addEventListener("focus", () => {
		renderMatches(filterCandidates(queryInput.value));
	});

	queryInput.addEventListener("keydown", (event) => {
		if (event.key === "Escape") {
			optionsContainer.hidden = true;
			queryInput.setAttribute("aria-expanded", "false");
			return;
		}
		if (event.key !== "Enter" || userIdInput.value) {
			return;
		}
		const firstMatch = filterCandidates(queryInput.value)[0];
		if (!firstMatch) {
			return;
		}
		event.preventDefault();
		applyCandidate(firstMatch);
	});

	queryInput.addEventListener("blur", () => {
		window.setTimeout(() => {
			optionsContainer.hidden = true;
			queryInput.setAttribute("aria-expanded", "false");
			const match = resolveCandidate(queryInput.value);
			queryInput.setCustomValidity(
				queryInput.value.trim() && !match ? "Choose an existing user." : "",
			);
		}, 100);
	});

	for (const candidate of candidates) {
		candidate.element.addEventListener("click", () => {
			applyCandidate(candidate);
			queryInput.focus();
		});
	}

	document.addEventListener("click", (event) => {
		if (!autocomplete.contains(event.target as Node)) {
			optionsContainer.hidden = true;
			queryInput.setAttribute("aria-expanded", "false");
		}
	});

	form.addEventListener("submit", (event) => {
		const match =
			syncSelection() ?? filterCandidates(queryInput.value)[0] ?? null;
		if (!match) {
			event.preventDefault();
			queryInput.setCustomValidity("Choose an existing user.");
			queryInput.reportValidity();
			return;
		}
		applyCandidate(match);
	});
};

const initEditor = () => {
	const root = document.getElementById("editor");
	const textarea = document.getElementById(
		"page_content",
	) as HTMLTextAreaElement | null;

	if (!root || !textarea) {
		return;
	}

	let syncingFromSource = false;
	let previewControls = {
		updatePreview: () => {},
		setSourceVisible: (_visible: boolean) => {},
		syncToolbarState: () => {},
	};

	try {
		const editor = new Editor({
			element: root,
			content: textarea.value || "",
			contentType: "markdown",
			extensions: [
				StarterKit.configure({
					link: false,
					heading: {
						levels: [1, 2, 3, 4, 5, 6],
					},
				}),
				Image.configure({ inline: true }),
				Link.configure({ openOnClick: false }),
				Markdown,
			],
			onUpdate: ({ editor }) => {
				if (syncingFromSource) {
					return;
				}
				textarea.value = editor.getMarkdown();
				previewControls.updatePreview();
			},
		});

		const assetModal = createAssetModal(editor);
		previewControls = bindToolbar(editor, textarea, assetModal.open);
		previewControls.updatePreview();
		previewControls.setSourceVisible(false);
		textarea.addEventListener("input", () => {
			syncingFromSource = true;
			try {
				editor.commands.setContent(textarea.value, {
					contentType: "markdown",
					emitUpdate: false,
				});
				previewControls.updatePreview();
				previewControls.syncToolbarState();
			} finally {
				syncingFromSource = false;
			}
		});
		document.body?.classList.add("editor-ready");
	} catch {
		const sourcePanel = document.querySelector<HTMLElement>(
			"[data-editor-source-panel]",
		);
		sourcePanel?.removeAttribute("hidden");
		document.body?.classList.add("editor-error");
	}
};

function doPageStartup() {
	initTransientMessages();
	initTagEditor();
	bindNewContentSlugController();
	initMembershipCreateForm();
	bindConfirmingForms();
	announceAssetLibraryUpdate();
	initEditor();

	const apiUrl = new URL("/", window.location.origin).href;
	OPENAPI_CLIENT = createClient<paths>({ baseUrl: apiUrl });
}

if (document.readyState === "loading") {
	document.addEventListener("DOMContentLoaded", () => {
		doPageStartup();
	});
} else {
	doPageStartup();
}
