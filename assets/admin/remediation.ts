import "./remediation.css";
import type { Client } from "openapi-fetch";
import createClient from "openapi-fetch";
import { ApiPaths, type components, type paths } from "./openapi";

let OPENAPI_CLIENT: Client<paths, `${string}/${string}`>;

type ManualAssetSelection = {
	asset_id: string;
	variant: string;
	asset_label: string;
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

const createAssetCard = (asset: components["schemas"]["AssetLibraryItem"]) => {
	const button = document.createElement("button");
	button.type = "button";
	button.className =
		"asset-card group grid gap-2 rounded-md border border-slate-200 bg-white p-2 text-left shadow-sm transition hover:border-slate-300 hover:bg-slate-50 focus:outline-none focus:ring-2 focus:ring-slate-200";
	button.dataset.assetPayload = JSON.stringify(asset);
	button.dataset.assetCard = "true";

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
	const dimensions =
		asset.width && asset.height ? `${asset.width}×${asset.height}` : "size n/a";
	meta.textContent = `${asset.mime_type} • ${dimensions} • ${formatAssetSize(asset.byte_length)} • ${formatAssetDate(asset.created_at)}`;

	button.appendChild(thumb);
	button.appendChild(name);
	button.appendChild(meta);
	return button;
};

const renderAssetGrid = (
	container: HTMLElement,
	assets: components["schemas"]["AssetLibraryItem"][],
	message: string,
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
		container.appendChild(createAssetCard(asset));
	}
};

const initRemediation = () => {
	const apiUrl = new URL("/", window.location.origin).href;
	OPENAPI_CLIENT = createClient<paths>({ baseUrl: apiUrl });

	const root = document.querySelector<HTMLElement>("[data-remediation-root]");
	if (!root) {
		return;
	}

	const siteId = root.dataset.siteId;
	const selectedIssuesInput = root.querySelector<HTMLInputElement>(
		"[data-selected-issues-json]",
	);
	const remoteImportInput = root.querySelector<HTMLInputElement>(
		"[data-remote-import-json]",
	);
	const assetSelectionsInput = root.querySelector<HTMLInputElement>(
		"[data-asset-selections-json]",
	);
	const modal = document.querySelector<HTMLElement>("[data-scan-asset-modal]");
	const searchInput = modal?.querySelector<HTMLInputElement>(
		"[data-scan-asset-search]",
	);
	const typeSelect = modal?.querySelector<HTMLSelectElement>(
		"[data-scan-asset-type]",
	);
	const sortBySelect = modal?.querySelector<HTMLSelectElement>(
		"[data-scan-asset-sort-by]",
	);
	const sortDirSelect = modal?.querySelector<HTMLSelectElement>(
		"[data-scan-asset-sort-dir]",
	);
	const variantSelect = modal?.querySelector<HTMLSelectElement>(
		"[data-scan-asset-variant]",
	);
	const recentSection = modal?.querySelector<HTMLElement>(
		"[data-scan-asset-recent-section]",
	);
	const resultsSection = modal?.querySelector<HTMLElement>(
		"[data-scan-asset-results-section]",
	);
	const recentGrid = modal?.querySelector<HTMLElement>(
		"[data-scan-asset-recent]",
	);
	const resultsGrid = modal?.querySelector<HTMLElement>(
		"[data-scan-asset-results]",
	);
	const applyButton = modal?.querySelector<HTMLButtonElement>(
		"[data-scan-asset-apply]",
	);
	const closeButtons = modal?.querySelectorAll<HTMLElement>(
		"[data-scan-asset-close]",
	);
	if (
		!siteId ||
		!selectedIssuesInput ||
		!remoteImportInput ||
		!assetSelectionsInput ||
		!modal ||
		!searchInput ||
		!typeSelect ||
		!sortBySelect ||
		!sortDirSelect ||
		!variantSelect ||
		!recentSection ||
		!resultsSection ||
		!recentGrid ||
		!resultsGrid ||
		!applyButton
	) {
		return;
	}

	let currentIssue: HTMLElement | null = null;
	let selectedAsset: components["schemas"]["AssetLibraryItem"] | null = null;
	let searchTimeout: number | null = null;
	const selections: Record<string, ManualAssetSelection> = {};

	const syncSelections = () => {
		assetSelectionsInput.value = JSON.stringify(selections);
	};

	const setModalOpen = (open: boolean) => {
		if (open) {
			modal.removeAttribute("hidden");
			modal.setAttribute("aria-hidden", "false");
			searchInput.focus();
			return;
		}
		modal.setAttribute("hidden", "");
		modal.setAttribute("aria-hidden", "true");
		currentIssue = null;
		selectedAsset = null;
		applyButton.disabled = true;
		modal.querySelectorAll<HTMLElement>("[data-asset-card]").forEach((card) => {
			card.classList.remove("border-slate-900", "ring-2", "ring-slate-300");
		});
	};

	const fetchAssets = async (query?: string) => {
		const { data, error } = await OPENAPI_CLIENT.GET(
			ApiPaths.api_site_assets_library,
			{
				params: {
					path: {
						site_id: siteId,
					},
					query: {
						q: query || "",
						limit: query ? 50 : 12,
						type: typeSelect.value || "",
						sort_by: sortBySelect.value || "uploaded",
						sort_dir: sortDirSelect.value || "desc",
					},
				},
			},
		);
		if (error || !data) {
			throw new Error("failed to load assets");
		}
		const payload = data as components["schemas"]["AssetLibraryResponse"];
		return payload.assets ?? [];
	};

	const loadRecent = async () => {
		renderAssetGrid(recentGrid, [], "Loading recent assets...");
		try {
			const assets = await fetchAssets();
			renderAssetGrid(recentGrid, assets, "No assets available.");
		} catch {
			renderAssetGrid(recentGrid, [], "Unable to load assets.");
		}
	};

	const loadSearch = async (query: string) => {
		renderAssetGrid(resultsGrid, [], "Searching...");
		try {
			const assets = await fetchAssets(query);
			renderAssetGrid(resultsGrid, assets, "No matches found.");
		} catch {
			renderAssetGrid(resultsGrid, [], "Unable to load assets.");
		}
	};

	const setSelectedAsset = (
		asset: components["schemas"]["AssetLibraryItem"] | null,
	) => {
		selectedAsset = asset;
		modal.querySelectorAll<HTMLElement>("[data-asset-card]").forEach((card) => {
			const selected =
				JSON.parse(card.getAttribute("data-asset-payload") ?? "{}").id ===
				asset?.id;
			card.classList.toggle("border-slate-900", selected);
			card.classList.toggle("ring-2", selected);
			card.classList.toggle("ring-slate-300", selected);
		});
		applyButton.disabled = !asset;
	};

	const openForIssue = (issue: HTMLElement) => {
		currentIssue = issue;
		selectedAsset = null;
		searchInput.value = "";
		sortBySelect.value = "uploaded";
		sortDirSelect.value = "desc";
		recentSection.removeAttribute("hidden");
		resultsSection.setAttribute("hidden", "");
		void loadRecent();
		setModalOpen(true);
	};

	const scheduleSearch = () => {
		if (searchTimeout) {
			window.clearTimeout(searchTimeout);
		}
		searchTimeout = window.setTimeout(() => {
			const query = searchInput.value.trim();
			if (!query) {
				recentSection.removeAttribute("hidden");
				resultsSection.setAttribute("hidden", "");
				void loadRecent();
				return;
			}
			recentSection.setAttribute("hidden", "");
			resultsSection.removeAttribute("hidden");
			void loadSearch(query);
		}, 250);
	};

	const updateIssueSelection = (
		issue: HTMLElement,
		selection: ManualAssetSelection | null,
	) => {
		const issueId = issue.dataset.issueId;
		if (!issueId) {
			return;
		}
		const label = issue.querySelector<HTMLElement>(
			"[data-selected-asset-label]",
		);
		const issueToggle = issue
			.closest(".scan-issue")
			?.querySelector<HTMLInputElement>("[data-issue-select]");
		if (!selection) {
			delete selections[issueId];
			if (label) {
				label.textContent = "No asset selected yet.";
			}
			if (issueToggle) {
				issueToggle.checked = false;
			}
			syncSelections();
			return;
		}
		selections[issueId] = selection;
		if (issueToggle) {
			issueToggle.checked = true;
		}
		if (label) {
			label.textContent = `${selection.asset_label} (${selection.variant})`;
		}
		syncSelections();
	};

	root
		.querySelectorAll<HTMLElement>("[data-remediation-issue]")
		.forEach((issue) => {
			const issueId = issue.dataset.issueId;
			const existing = issue.dataset.selectedAsset;
			const existingLabel = issue.dataset.selectedLabel;
			if (issueId && existing) {
				const [assetId, variant] = existing.split(":");
				selections[issueId] = {
					asset_id: assetId,
					variant,
					asset_label: existingLabel ?? assetId,
				};
			}
			issue
				.querySelector<HTMLElement>("[data-pick-asset]")
				?.addEventListener("click", () => openForIssue(issue));
		});
	syncSelections();
	root.addEventListener("submit", (event) => {
		const submitter =
			"submitter" in event ? (event as SubmitEvent).submitter : undefined;
		const remoteImportButton =
			submitter instanceof HTMLButtonElement
				? submitter.closest<HTMLButtonElement>("[data-remote-import-submit]")
				: null;
		if (remoteImportButton?.value) {
			const issueId = remoteImportButton.value;
			selectedIssuesInput.value = JSON.stringify([issueId]);
			remoteImportInput.value = JSON.stringify([issueId]);
			assetSelectionsInput.value = JSON.stringify(
				selections[issueId] ? { [issueId]: selections[issueId] } : {},
			);
			return;
		}
		const selectedIssueIds = Array.from(
			root.querySelectorAll<HTMLInputElement>("[data-issue-select]:checked"),
		).map((input) => input.value);
		selectedIssuesInput.value = JSON.stringify(selectedIssueIds);
		remoteImportInput.value = JSON.stringify([]);
		syncSelections();
	});

	modal.addEventListener("click", (event) => {
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
		const parsed = JSON.parse(
			payload,
		) as components["schemas"]["AssetLibraryItem"];
		setSelectedAsset(parsed);
	});

	applyButton.addEventListener("click", () => {
		if (!currentIssue || !selectedAsset) {
			return;
		}
		updateIssueSelection(currentIssue, {
			asset_id: selectedAsset.id,
			variant: variantSelect.value,
			asset_label: selectedAsset.original_filename,
		});
		setModalOpen(false);
	});

	searchInput.addEventListener("input", scheduleSearch);
	typeSelect.addEventListener("change", scheduleSearch);
	sortBySelect.addEventListener("change", scheduleSearch);
	sortDirSelect.addEventListener("change", scheduleSearch);
	closeButtons?.forEach((button) => {
		button.addEventListener("click", () => setModalOpen(false));
	});
};

if (document.readyState === "loading") {
	document.addEventListener("DOMContentLoaded", initRemediation);
} else {
	initRemediation();
}
