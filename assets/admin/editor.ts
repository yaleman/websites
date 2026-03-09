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

	const insertImageMarkup = (options: {
		src: string;
		alt?: string;
		href?: string;
	}) => {
		const image = document.createElement("img");
		image.src = options.src;
		if (options.alt) {
			image.alt = options.alt;
		}
		if (!options.href) {
			editor.chain().focus().insertContent(image.outerHTML).run();
			return;
		}

		const link = document.createElement("a");
		link.href = options.href;
		link.appendChild(image);
		editor.chain().focus().insertContent(link.outerHTML).run();
	};

	const insertSelection = () => {
		const altText = altInput?.value.trim();
		const externalUrl = externalInput?.value.trim();
		if (externalUrl) {
			insertImageMarkup({ src: externalUrl, alt: altText });
			close();
			return;
		}

		if (!selectedAsset) {
			return;
		}
		if (selectedAsset.has_thumbnail && selectedAsset.thumbnail_url) {
			insertImageMarkup({
				src: selectedAsset.thumbnail_url,
				alt: altText,
				href: selectedAsset.original_url,
			});
		} else {
			insertImageMarkup({
				src: selectedAsset.original_url,
				alt: altText,
			});
		}
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
		if (altInput) {
			altInput.value = "";
		}
		if (externalInput) {
			externalInput.value = "";
		}
		setSectionVisibility(false);
		clearSelection();
	};

	const open = () => {
		resetModalState();
		setModalOpen(true);
		loadRecent();
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

const initTagEditor = () => {
	const form = document.querySelector<HTMLFormElement>("form.editor-form");
	const input = document.querySelector<HTMLInputElement>("[data-tag-input]");
	const hiddenInput = document.querySelector<HTMLInputElement>("[data-tag-list]");
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

	const selectedTags = Array.from(chipContainer.querySelectorAll<HTMLElement>("[data-tag-chip]"))
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
			chip.className = "tag-chip";
			chip.dataset.tagChip = tag;

			const label = document.createElement("span");
			label.textContent = tag;
			chip.appendChild(label);

			const remove = document.createElement("span");
			remove.className = "tag-chip__remove";
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
		if (selectedTags.some((existing) => existing.toLowerCase() === tag.toLowerCase())) {
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
	const form = document.querySelector<HTMLFormElement>("[data-membership-create]");
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
	const emptyState = form?.querySelector<HTMLElement>("[data-membership-empty]");

	if (!form || !autocomplete || !queryInput || !userIdInput || !optionsContainer || !emptyState) {
		return;
	}

	const candidates = Array.from(
		optionsContainer.querySelectorAll<HTMLButtonElement>("[data-membership-option]"),
	)
		.map((option) => ({
			element: option,
			value: option.dataset.searchValue?.trim() ?? "",
			userId: option.dataset.userId ?? "",
			subject: option.dataset.userSubject?.trim() ?? "",
			email: option.dataset.userEmail?.trim() ?? "",
		}))
		.filter((candidate) => candidate.value.length > 0 && candidate.userId.length > 0);

	const normalize = (value: string) => value.trim().toLowerCase();

	const resolveCandidate = (rawValue: string) => {
		const normalized = normalize(rawValue);
		if (!normalized) {
			return null;
		}
		return (
			candidates.find((candidate) => normalize(candidate.value) === normalized) ??
			candidates.find((candidate) => normalize(candidate.subject) === normalized) ??
			candidates.find((candidate) => normalize(candidate.email) === normalized) ??
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
		const match = syncSelection() ?? filterCandidates(queryInput.value)[0] ?? null;
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
		initTagEditor();
		initMembershipCreateForm();
		initEditor();
	});
} else {
	initTransientMessages();
	initTagEditor();
	initMembershipCreateForm();
	initEditor();
}
