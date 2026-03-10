import { defineConfig } from "@playwright/test";

export default defineConfig({
	testDir: "./tests/e2e",
	timeout: 60_000,
	globalSetup: "./tests/e2e/global_setup.ts",
	use: {
		headless: true,
		baseURL: process.env.E2E_BASE_URL,
	},
	projects: [
		{
			name: "chromium",
			use: { browserName: "chromium" },
		},
	],
});
