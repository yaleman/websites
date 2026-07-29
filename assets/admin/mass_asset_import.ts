const initMassAssetImport = () => {
	const root = document.querySelector<HTMLElement>(
		"[data-mass-asset-import-root]",
	);
	if (!root?.dataset.siteId) {
		return;
	}

	const siteId = root.dataset.siteId;
	for (const button of root.querySelectorAll<HTMLButtonElement>(
		"[data-recheck-mass-asset]",
	)) {
		button.addEventListener("click", async () => {
			const row = button.closest<HTMLElement>("[data-mass-asset-row]");
			const status = row?.querySelector<HTMLElement>("[data-recheck-status]");
			const path = row?.dataset.normalizedPath;
			if (!row || !status || !path) {
				return;
			}
			button.disabled = true;
			status.textContent = "Checking...";
			try {
				const response = await fetch(
					`/admin/site/${siteId}/assets/mass-import/recheck`,
					{
						method: "POST",
						headers: {
							"Content-Type": "application/json",
						},
						body: JSON.stringify({ path }),
					},
				);
				if (!response.ok) {
					throw new Error("recheck failed");
				}
				const payload = (await response.json()) as {
					complete: boolean;
					occurrence_count: number;
				};
				if (payload.complete) {
					row.classList.add("opacity-60");
					status.textContent = "Complete";
				} else {
					status.textContent = `${payload.occurrence_count} remaining`;
				}
			} catch {
				status.textContent = "Unable to recheck";
			} finally {
				button.disabled = false;
			}
		});
	}
};

document.addEventListener("DOMContentLoaded", initMassAssetImport);
