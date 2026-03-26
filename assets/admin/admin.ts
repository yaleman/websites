import "./styles.css";

type SiteImportLookupResponse = {
	short_name: string;
	exists: boolean;
	full_title: string | null;
};

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
		submit.disabled = false;
		submit.textContent = "Import site";
		setStatus(null);
	};

	const showDuplicatePrompt = (lookup: SiteImportLookupResponse) => {
		prompt.hidden = false;
		details.textContent = lookup.full_title
			? `Site ${lookup.short_name} already exists as ${lookup.full_title}. Importing will replace the existing site.`
			: `Site ${lookup.short_name} already exists. Importing will replace the existing site.`;
		replace.checked = false;
		submit.disabled = true;
		submit.textContent = "Replace site and import";
		setStatus(null);
	};

	replace.addEventListener("change", () => {
		submit.disabled = prompt.hidden || !replace.checked;
		submit.textContent = replace.checked
			? "Replace site and import"
			: "Import site";
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
			submit.disabled = true;
			submit.textContent = "Checking existing site";
			setStatus(null);

			const response = await fetch(
				`${checkUrl}?short_name=${encodeURIComponent(shortName)}`,
				{
					headers: {
						Accept: "application/json",
					},
				},
			);
			if (serial !== requestSerial) {
				return;
			}
			if (!response.ok) {
				setStatus(
					"Could not verify whether this site already exists. You can still submit the import.",
				);
				return;
			}

			const lookup = (await response.json()) as SiteImportLookupResponse;
			if (lookup.exists) {
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

initSiteImportPrompt();
