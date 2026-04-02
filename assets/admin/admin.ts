import "./styles.css";

type SiteImportLookupResponse = {
	short_name: string;
	exists: boolean;
	full_title: string | null;
};

type SiteImportLookupSource = {
	short_name: string;
	exists: boolean;
	full_title: string | null;
};

const SITE_IMPORT_LOOKUP_TIMEOUT_MS = 5000;

const initSiteImportPrompt = () => {
	const root = document.querySelector<HTMLElement>("[data-site-import-root]");
	const form = root?.querySelector<HTMLFormElement>("[data-site-import-form]");
	const fileInput =
		form?.querySelector<HTMLInputElement>("[data-site-import-file]");
	const prompt = form?.querySelector<HTMLElement>("[data-site-import-prompt]");
	const details = form?.querySelector<HTMLElement>("[data-site-import-details]");
	const replace = form?.querySelector<HTMLInputElement>(
		"[data-site-import-replace]",
	);
	const status = form?.querySelector<HTMLElement>("[data-site-import-status]");
	const submit = form?.querySelector<HTMLButtonElement>(
		"[data-site-import-submit]",
	);
	const checkUrl = root?.dataset.checkUrl;

	if (!root || !form || !fileInput || !prompt || !details || !replace || !submit || !checkUrl) {
		return;
	}

	let requestSerial = 0;
	let duplicateConfirmationRequired = false;
	let lookupPending = false;

	const syncSubmitState = () => {
		if (lookupPending) {
			submit.disabled = true;
			submit.textContent = "Checking existing site";
			return;
		}
		if (duplicateConfirmationRequired) {
			submit.disabled = !replace.checked;
			submit.textContent = "Replace site and import";
			return;
		}
		submit.disabled = false;
		submit.textContent = "Import site";
	};

	const setStatus = (message: string | null) => {
		if (!status) {
			return;
		}
		status.textContent = message ?? "";
		status.hidden = !message;
	};

	const resetPrompt = () => {
		prompt.hidden = true;
		replace.checked = false;
		duplicateConfirmationRequired = false;
		lookupPending = false;
		syncSubmitState();
		setStatus(null);
	};

	const showDuplicatePrompt = (lookup: SiteImportLookupResponse) => {
		prompt.hidden = false;
		details.textContent = lookup.full_title
			? `Site ${lookup.short_name} already exists as ${lookup.full_title}. Importing will replace the existing site.`
			: `Site ${lookup.short_name} already exists. Importing will replace the existing site.`;
		duplicateConfirmationRequired = true;
		lookupPending = false;
		syncSubmitState();
		setStatus(null);
	};

	const lookupSiteOnDashboard = async (
		shortName: string,
	): Promise<SiteImportLookupSource | null> => {
		const response = await fetch("/admin", {
			headers: {
				Accept: "text/html",
			},
		});
		if (!response.ok) {
			return null;
		}

		const document = new DOMParser().parseFromString(
			await response.text(),
			"text/html",
		);
		for (const row of document.querySelectorAll("tbody tr")) {
			const links = row.querySelectorAll("a");
			if (links.length < 2) {
				continue;
			}

			const rowShortName = links[1].textContent?.trim();
			if (rowShortName !== shortName) {
				continue;
			}

			return {
				short_name: rowShortName,
				exists: true,
				full_title: links[0].textContent?.trim() ?? null,
			};
		}

		return {
			short_name: shortName,
			exists: false,
			full_title: null,
		};
	};

	const lookupExistingSite = async (
		shortName: string,
	): Promise<SiteImportLookupSource | null> => {
		const controller = new AbortController();
		const timeout = window.setTimeout(() => {
			controller.abort();
		}, SITE_IMPORT_LOOKUP_TIMEOUT_MS);

		try {
			const response = await fetch(
				`${checkUrl}?short_name=${encodeURIComponent(shortName)}`,
				{
					headers: {
						Accept: "application/json",
					},
					signal: controller.signal,
				},
			);
			if (response.ok) {
				return (await response.json()) as SiteImportLookupSource;
			}

			return lookupSiteOnDashboard(shortName);
		} catch {
			// Fall back to the dashboard HTML if the lookup endpoint is unavailable.
		} finally {
			window.clearTimeout(timeout);
		}

		return lookupSiteOnDashboard(shortName);
	};

	replace.addEventListener("change", () => {
		syncSubmitState();
	});

	form.addEventListener("submit", (event) => {
		if (!prompt.hidden && !replace.checked) {
			event.preventDefault();
			replace.focus();
		}
	});

	fileInput.addEventListener("change", async () => {
		const file = fileInput.files?.[0];
		if (!file) {
			resetPrompt();
			return;
		}

		const serial = ++requestSerial;
		try {
			const parsed = JSON.parse(await file.text()) as {
				site?: { short_name?: unknown };
			};
			const shortName =
				typeof parsed.site?.short_name === "string"
					? parsed.site.short_name.trim()
					: "";
			if (!shortName) {
				resetPrompt();
				setStatus("The selected file does not look like a site export.");
				return;
			}

			prompt.hidden = false;
			details.textContent = `Checking whether site ${shortName} already exists...`;
			replace.checked = false;
			duplicateConfirmationRequired = true;
			lookupPending = true;
			syncSubmitState();
			setStatus(null);

			const lookup = await lookupExistingSite(shortName);
			if (serial !== requestSerial) {
				return;
			}
			if (!lookup) {
				prompt.hidden = true;
				duplicateConfirmationRequired = false;
				lookupPending = false;
				syncSubmitState();
				setStatus(
					"Could not verify whether this site already exists. You can still submit the import.",
				);
				return;
			}
			if (lookup?.exists) {
				showDuplicatePrompt(lookup);
			} else {
				resetPrompt();
			}
		} catch {
			if (serial !== requestSerial) {
				return;
			}
			resetPrompt();
			setStatus("The selected file could not be read as site export JSON.");
		}
	});
};

const initPublishMethodSwitcher = () => {
	const select = document.querySelector<HTMLSelectElement>(
		"[data-publish-method-select]",
	);
	const panels = Array.from(
		document.querySelectorAll<HTMLElement>("[data-publish-method-panel]"),
	);

	if (!select || panels.length === 0) {
		return;
	}

	const updatePanels = () => {
		const selectedMethod = select.value;
		for (const panel of panels) {
			const matches = panel.dataset.publishMethodPanel === selectedMethod;
			panel.hidden = !matches;
			panel.setAttribute("aria-hidden", String(!matches));
		}
	};

	select.addEventListener("change", updatePanels);
	updatePanels();
};

initSiteImportPrompt();
initPublishMethodSwitcher();
