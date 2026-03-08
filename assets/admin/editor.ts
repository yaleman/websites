import { Editor } from "@tiptap/core";
import Image from "@tiptap/extension-image";
import Link from "@tiptap/extension-link";
import { Markdown } from "@tiptap/markdown";
import StarterKit from "@tiptap/starter-kit";
import "./editor.css";
import "./styles.css";

type AssetLibraryItem = {
	id: string;
	original_filename: string;
	mime_type: string;
	width: number | null;
	height: number | null;
	created_at: string;
	original_url: string;
	thumbnail_url: string | null;
	has_thumbnail: boolean;
};

const inferAltFromFilename = (filename: string) => {
	const trimmed = filename.replace(/\.[^/.]+$/, "");
	return trimmed.replace(/[-_]+/g, " ").replace(/\s+/g, " ").trim();
};

const formatAssetMeta = (asset: AssetLibraryItem) => {
	const dimensions =
		asset.width && asset.height ? `${asset.width}×${asset.height}` : "size n/a";
	return `${asset.mime_type} • ${dimensions}`;
};

const createAssetCard = (asset: AssetLibraryItem) => {
	const button = document.createElement("button");
	button.type = "button";
	button.className = "asset-card";
	button.dataset.assetId = asset.id;
	button.dataset.assetPayload = JSON.stringify(asset);

	const thumb = document.createElement("div");
	thumb.className = "asset-card__thumb";
	const image = document.createElement("img");
	image.src = asset.thumbnail_url ?? asset.original_url;
	image.alt = "";
	image.loading = "lazy";
	thumb.appendChild(image);

	const name = document.createElement("div");
	name.className = "asset-card__name";
	name.textContent = asset.original_filename;

	const meta = document.createElement("div");
	meta.className = "asset-card__meta";
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
) => {
	container.innerHTML = "";
	if (assets.length === 0) {
		const empty = document.createElement("div");
		empty.className = "asset-empty";
		empty.textContent = message;
		container.appendChild(empty);
		return;
	}
	for (const asset of assets) {
		container.appendChild(createAssetCard(asset));
	}
};

const createAssetModal = (editor: Editor) => {
	const modal = document.querySelector<HTMLElement>("[data-asset-modal]");
	const configRoot = document.querySelector<HTMLElement>("[data-editor-config]");
	const siteId = configRoot?.dataset.siteId;

	if (!modal || !siteId) {
		return { open: () => {} };
	}

	const closeButtons = modal.querySelectorAll<HTMLElement>(
		"[data-asset-modal-close]",
	);
	const searchInput = modal.querySelector<HTMLInputElement>("[data-asset-search]");
	const typeSelect = modal.querySelector<HTMLSelectElement>("[data-asset-type]");
	const recentSection = modal.querySelector<HTMLElement>(
		"[data-asset-recent-section]",
	);
	const resultsSection = modal.querySelector<HTMLElement>(
		"[data-asset-results-section]",
	);
	const recentGrid = modal.querySelector<HTMLElement>("[data-asset-recent]");
	const resultsGrid = modal.querySelector<HTMLElement>("[data-asset-results]");
	const altInput = modal.querySelector<HTMLInputElement>("[data-asset-alt]");
	const externalInput =
		modal.querySelector<HTMLInputElement>("[data-asset-external]");
	const insertButton = modal.querySelector<HTMLButtonElement>(
		"[data-asset-insert]",
	);

	let selectedAsset: AssetLibraryItem | null = null;
	let searchTimeout: number | null = null;

	const setModalOpen = (open: boolean) => {
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

	const setInsertEnabled = () => {
		if (!insertButton) {
			return;
		}
		const externalUrl = externalInput?.value.trim();
		insertButton.disabled = !selectedAsset && !externalUrl;
	};

	const clearSelection = () => {
		selectedAsset = null;
		modal.querySelectorAll(".asset-card.is-selected").forEach((card) => {
			card.classList.remove("is-selected");
		});
		setInsertEnabled();
	};

	const setSelectedAsset = (asset: AssetLibraryItem | null) => {
		selectedAsset = asset;
		modal.querySelectorAll(".asset-card").forEach((card) => {
			card.classList.toggle(
				"is-selected",
				card.getAttribute("data-asset-id") === asset?.id,
			);
		});
		if (asset && altInput && !altInput.value.trim()) {
			altInput.value = inferAltFromFilename(asset.original_filename);
		}
		setInsertEnabled();
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
	}) => {
		const url = new URL(
			`/admin/site/${siteId}/assets/library`,
			window.location.origin,
		);
		if (options.query) {
			url.searchParams.set("q", options.query);
		}
		if (options.type) {
			url.searchParams.set("type", options.type);
		}
		url.searchParams.set("limit", options.limit.toString());
		const response = await fetch(url.toString(), { credentials: "same-origin" });
		if (!response.ok) {
			throw new Error("Failed to fetch assets.");
		}
		const payload = (await response.json()) as { assets: AssetLibraryItem[] };
		return payload.assets ?? [];
	};

	const loadRecent = async () => {
		if (!recentGrid) {
			return;
		}
		renderAssetGrid(recentGrid, [], "Loading recent images...");
		clearSelection();
		try {
			const assets = await fetchAssets({
				limit: 12,
				type: typeSelect?.value ?? "all",
			});
			renderAssetGrid(recentGrid, assets, "No images uploaded yet.");
			clearSelection();
		} catch {
			renderAssetGrid(recentGrid, [], "Unable to load recent images.");
			clearSelection();
		}
	};

	const loadSearch = async (query: string) => {
		if (!resultsGrid) {
			return;
		}
		renderAssetGrid(resultsGrid, [], "Searching...");
		clearSelection();
		try {
			const assets = await fetchAssets({
				query,
				limit: 50,
				type: typeSelect?.value ?? "all",
			});
			renderAssetGrid(resultsGrid, assets, "No matches found.");
			clearSelection();
		} catch {
			renderAssetGrid(resultsGrid, [], "Unable to load search results.");
			clearSelection();
		}
	};

	const scheduleSearch = () => {
		const query = searchInput?.value.trim() ?? "";
		if (searchTimeout) {
			window.clearTimeout(searchTimeout);
		}
		searchTimeout = window.setTimeout(() => {
			if (query) {
				setSectionVisibility(true);
				loadSearch(query);
			} else {
				setSectionVisibility(false);
				loadRecent();
			}
		}, 300);
	};

	const handleAssetClick = (event: Event) => {
		const target = event.target as HTMLElement | null;
		if (!target) {
			return;
		}
		const card = target.closest<HTMLButtonElement>(".asset-card");
		if (!card) {
			return;
		}
		const payload = card.dataset.assetPayload;
		if (!payload) {
			return;
		}
		const asset = JSON.parse(payload) as AssetLibraryItem;
		setSelectedAsset(asset);
	};

	const insertSelection = () => {
		const altText = altInput?.value.trim();
		const externalUrl = externalInput?.value.trim();
		if (externalUrl) {
			const attrs = altText ? { src: externalUrl, alt: altText } : { src: externalUrl };
			editor.chain().focus().setImage(attrs).run();
			close();
			return;
		}

		if (!selectedAsset) {
			return;
		}
		if (selectedAsset.has_thumbnail && selectedAsset.thumbnail_url) {
			editor
				.chain()
				.focus()
				.insertContent({
					type: "image",
					attrs: {
						src: selectedAsset.thumbnail_url,
						alt: altText || null,
					},
					marks: [
						{
							type: "link",
							attrs: {
								href: selectedAsset.original_url,
							},
						},
					],
				})
				.run();
		} else {
			const attrs = altText
				? { src: selectedAsset.original_url, alt: altText }
				: { src: selectedAsset.original_url };
			editor.chain().focus().setImage(attrs).run();
		}
		close();
	};

	const open = () => {
		clearSelection();
		if (searchInput) {
			searchInput.value = "";
		}
		if (altInput) {
			altInput.value = "";
		}
		if (externalInput) {
			externalInput.value = "";
		}
		setSectionVisibility(false);
		setModalOpen(true);
		loadRecent();
	};

	const close = () => {
		setModalOpen(false);
		clearSelection();
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

	searchInput?.addEventListener("input", scheduleSearch);
	searchInput?.addEventListener("keydown", (event) => {
		if (event.key === "Enter") {
			event.preventDefault();
			scheduleSearch();
		}
	});
	typeSelect?.addEventListener("change", scheduleSearch);
	externalInput?.addEventListener("input", setInsertEnabled);
	externalInput?.addEventListener("keydown", (event) => {
		if (event.key === "Enter") {
			event.preventDefault();
			insertSelection();
		}
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

	const updatePreview = () => {
		if (!previewBody) {
			return;
		}
		previewBody.innerHTML = editor.getHTML();
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
	};

	if (!toolbar) {
		return { updatePreview, setSourceVisible };
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
			case "h2":
				editor.chain().focus().toggleHeading({ level: 2 }).run();
				return;
			case "h3":
				editor.chain().focus().toggleHeading({ level: 3 }).run();
				return;
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

	toolbar.addEventListener("click", (event) => {
		const target = event.target as HTMLElement | null;
		if (!target) {
			return;
		}

		const button = target.closest<HTMLButtonElement>("button[data-command]");
		if (!button) {
			return;
		}

		const commandName = button.getAttribute("data-command");
		if (!commandName) {
			return;
		}

		event.preventDefault();
		handleCommand(commandName);
	});

	return { updatePreview, setSourceVisible };
};

const initTransientMessages = () => {
	const messages = document.querySelectorAll<HTMLElement>("[data-auto-dismiss-ms]");

	for (const message of messages) {
		const dismissMs = Number.parseInt(
			message.dataset.autoDismissMs ?? "",
			10,
		);
		if (!Number.isFinite(dismissMs) || dismissMs <= 0) {
			continue;
		}

		const queryParam = message.dataset.clearQueryParam;
		if (queryParam) {
			const url = new URL(window.location.href);
			if (url.searchParams.has(queryParam)) {
				url.searchParams.delete(queryParam);
				const nextUrl = `${url.pathname}${url.search}${url.hash}`;
				window.history.replaceState({}, "", nextUrl);
			}
		}

		window.setTimeout(() => {
			message.setAttribute("hidden", "");
		}, dismissMs);
	}
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
	};

	try {
		const editor = new Editor({
			element: root,
			content: textarea.value || "",
			contentType: "markdown",
			extensions: [
				StarterKit.configure({ link: false }),
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

if (document.readyState === "loading") {
	document.addEventListener("DOMContentLoaded", () => {
		initTransientMessages();
		initEditor();
	});
} else {
	initTransientMessages();
	initEditor();
}
