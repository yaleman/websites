import path from "node:path";

import { runCommand } from "./support";

export default async function globalSetup(): Promise<void> {
	const manifestPath = path.join(process.cwd(), "Cargo.toml");
	await runCommand("cargo", [
		"build",
		"--manifest-path",
		manifestPath,
		"--bin",
		"websites",
		"--bin",
		"session_seed",
	]);
}
