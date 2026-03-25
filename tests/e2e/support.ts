import {
	type APIRequestContext,
	type Browser,
	type BrowserContext,
	type Page,
	request as playwrightRequest,
} from "@playwright/test";
import { spawn } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, rm, symlink } from "node:fs/promises";
import net from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";

export type CommandResult = {
	stdout: string;
	stderr: string;
};

export type SiteRole = "viewer" | "author" | "editor" | "owner";

type ServerProcess = ReturnType<typeof spawn>;

type RevisionRow = {
	id: string;
	revisionNumber: number;
	title: string;
	draft: boolean;
	createdAt: string;
};

export type AuditEventRow = {
	id: string;
	siteId: string | null;
	actorSub: string;
	eventType: string;
	entityType: string;
	entityId: string;
	createdAt: string;
	payloadJson: unknown;
};

type CreateContentInput = {
	siteId?: string;
	pageType: string;
	title: string;
	slug: string;
	pageContent: string;
	creatorSub: string;
	draft?: boolean;
};

type UpdateContentInput = {
	contentId: string;
	pageType?: string;
	title?: string;
	slug?: string;
	pageContent?: string;
	draft?: boolean;
	publishedAt?: string;
	editorSub: string;
};

export type TestHarness = {
	tempRoot: string;
	uploadRoot: string;
	dbPath: string;
	databaseUrl: string;
	baseUrl: string;
	env: NodeJS.ProcessEnv;
	tlsCertPath: string;
	tlsKeyPath: string;
	port: number;
	server: ServerProcess;
	serverLogs: { stdout: string; stderr: string };
	siteId: string;
};

export type AuthenticatedPage = {
	context: BrowserContext;
	page: Page;
	sessionId: string;
};

export type AuthenticatedApiContext = {
	api: APIRequestContext;
	sessionId: string;
};

export type AuthorizationFixtures = {
	ownerSubject: string;
	ownerUserId: string;
	ownerMembershipId: string;
	contentId: string;
	revisionId: string;
	tagId: string;
	tagName: string;
	assetId: string;
	assetOriginalFilename: string;
	membershipTargetSubject: string;
	membershipTargetUserId: string;
	membershipTargetId: string;
	previewAssetPath: string;
	siteFullTitle: string;
	templateName: string;
};

const oidcTestArgs = [
	"--client-id",
	"playwright-test-client",
	"--discovery-url",
	"https://example.com/.well-known/openid-configuration",
] as const;
const workspaceRoot = process.cwd();
const executableSuffix = process.platform === "win32" ? ".exe" : "";
const websitesBinaryPath = path.join(
	workspaceRoot,
	"target",
	"debug",
	`websites${executableSuffix}`,
);
const sessionSeedBinaryPath = path.join(
	workspaceRoot,
	"target",
	"debug",
	`session_seed${executableSuffix}`,
);

export const tinyPngBytes = Buffer.from(
	"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4////fwAJ+wP9KobjigAAAABJRU5ErkJggg==",
	"base64",
);

export function runCommand(
	command: string,
	args: string[],
	options: { env?: NodeJS.ProcessEnv; cwd?: string } = {},
): Promise<CommandResult> {
	return new Promise((resolve, reject) => {
		const child = spawn(command, args, {
			env: options.env ?? process.env,
			cwd: options.cwd,
			stdio: ["ignore", "pipe", "pipe"],
		});
		let stdout = "";
		let stderr = "";

		child.stdout.on("data", (data) => {
			stdout += data.toString();
		});
		child.stderr.on("data", (data) => {
			stderr += data.toString();
		});

		child.on("error", (err) => {
			reject(err);
		});
		child.on("close", (code) => {
			if (code === 0) {
				resolve({ stdout, stderr });
			} else {
				reject(
					new Error(
						`command failed (${command} ${args.join(" ")}): ${stderr}\n${stdout}`,
					),
				);
			}
		});
	});
}

function runWebsitesCommand(
	args: string[],
	options: { env?: NodeJS.ProcessEnv; cwd?: string } = {},
): Promise<CommandResult> {
	return runCommand(websitesBinaryPath, args, options);
}

function runSessionSeedCommand(
	args: string[],
	options: { env?: NodeJS.ProcessEnv; cwd?: string } = {},
): Promise<CommandResult> {
	return runCommand(sessionSeedBinaryPath, args, options);
}

async function loadEnvValue(name: string): Promise<string | undefined> {
	const value = process.env[name];
	if (value && value.length > 0) {
		return value;
	}

	try {
		const envrc = await readFile(".envrc", "utf8");
		const pattern = new RegExp(`export\\s+${name}=(?:"([^"]+)"|([^\\s]+))`);
		const match = envrc.match(pattern);
		return match?.[1] ?? match?.[2];
	} catch {
		return undefined;
	}
}

async function reservePort(): Promise<number> {
	return new Promise((resolve, reject) => {
		const server = net.createServer();
		server.on("error", reject);
		server.listen(0, "127.0.0.1", () => {
			const address = server.address();
			if (!address || typeof address === "string") {
				server.close();
				reject(new Error("failed to reserve port"));
				return;
			}
			const port = address.port;
			server.close(() => resolve(port));
		});
	});
}

async function waitForPort(
	port: number,
	server: ServerProcess,
	logs: { stdout: string; stderr: string },
	timeoutMs: number,
): Promise<void> {
	const startedAt = Date.now();
	while (Date.now() - startedAt < timeoutMs) {
		if (server.exitCode !== null) {
			throw new Error(`server exited early:\n${logs.stderr}\n${logs.stdout}`);
		}

		try {
			await new Promise<void>((resolve, reject) => {
				const socket = net.connect(port, "127.0.0.1", () => {
					socket.end();
					resolve();
				});
				socket.once("error", reject);
			});
			return;
		} catch {
			await new Promise((resolve) => setTimeout(resolve, 250));
		}
	}

	throw new Error(
		`server did not become ready in time:\n${logs.stderr}\n${logs.stdout}`,
	);
}

async function resolveTlsPaths(): Promise<{
	tlsCertPath: string;
	tlsKeyPath: string;
}> {
	const tlsCertPath = await loadEnvValue("WEBSITES_TLS_CERT_PATH");
	const tlsKeyPath = await loadEnvValue("WEBSITES_TLS_KEY_PATH");
	if (!tlsCertPath || !tlsKeyPath) {
		throw new Error(
			"Missing WEBSITES_TLS_CERT_PATH/WEBSITES_TLS_KEY_PATH. Set env vars or update .envrc.",
		);
	}
	return { tlsCertPath, tlsKeyPath };
}

function parseCreatedId(stdout: string, label: string): string {
	const match = stdout.match(new RegExp(`created ${label}: ([^ ]+)`));
	if (!match) {
		throw new Error(`failed to parse ${label} id: ${stdout}`);
	}
	return match[1];
}

function parseRevisions(stdout: string): RevisionRow[] {
	const lines = stdout
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line.length > 0);
	const header = "id\trevision_number\ttitle\tdraft\tcreated_at";
	const headerIndex = lines.indexOf(header);

	if (headerIndex === -1 || headerIndex === lines.length - 1) {
		return [];
	}

	return lines
		.slice(headerIndex + 1)
		.map((line) => line.split("\t"))
		.filter((columns) => columns.length >= 5)
		.map(([id, revisionNumber, title, draft, createdAt]) => ({
			id,
			revisionNumber: Number.parseInt(revisionNumber, 10),
			title,
			draft: draft === "true",
			createdAt,
		}));
}

function parseAuditEvents(stdout: string): AuditEventRow[] {
	const lines = stdout
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line.length > 0);
	const header =
		"id\tsite_id\tactor_sub\tevent_type\tentity_type\tentity_id\tcreated_at\tpayload_json";
	const headerIndex = lines.findIndex((line) => line === header);

	if (headerIndex === -1 || headerIndex === lines.length - 1) {
		return [];
	}

	return lines
		.slice(headerIndex + 1)
		.map((line) => line.split("\t"))
		.filter((columns) => columns.length >= 7)
		.map((columns) => {
			const [
				id,
				siteId,
				actorSub,
				eventType,
				entityType,
				entityId,
				createdAt,
				...payloadColumns
			] = columns;
			const payloadJson = payloadColumns.join("\t");

			return {
				id,
				siteId: siteId === "-" ? null : siteId,
				actorSub,
				eventType,
				entityType,
				entityId,
				createdAt,
				payloadJson:
					payloadJson.length === 0 || payloadJson === "null"
						? null
						: JSON.parse(payloadJson),
			};
		});
}

export function extractCsrfToken(html: string): string {
	const match = html.match(/name="csrf_token"\s+value="([^"]+)"/);
	if (!match) {
		throw new Error(`failed to find csrf token: ${html}`);
	}
	return match[1];
}

export async function setupHarness(): Promise<TestHarness> {
	const tempRoot = await mkdtemp(path.join(tmpdir(), "websites-e2e-"));
	const uploadRoot = path.join(tempRoot, "uploads");
	const siteTemplatesRoot = path.join(tempRoot, "site_templates");
	const dbPath = path.join(tempRoot, "database.sqlite");
	const databaseUrl = `sqlite://${dbPath}?mode=rwc`;
	const env = { ...process.env, WEBSITES_UPLOAD_ROOT: uploadRoot };
	const { tlsCertPath, tlsKeyPath } = await resolveTlsPaths();

	await symlink(
		path.join(workspaceRoot, "admin-ui-assets"),
		path.join(tempRoot, "admin-ui-assets"),
	);
	await mkdir(siteTemplatesRoot, { recursive: true });
	await cp(
		path.join(workspaceRoot, "site_templates", "default"),
		path.join(siteTemplatesRoot, "default"),
		{
			recursive: true,
		},
	);

	await runWebsitesCommand(
		[
			"--database-url",
			dbPath,
			"--tls-cert-path",
			tlsCertPath,
			"--tls-key-path",
			tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"init",
		],
		{ cwd: tempRoot, env },
	);
	const siteResult = await runWebsitesCommand(
		[
			"--database-url",
			dbPath,
			"--tls-cert-path",
			tlsCertPath,
			"--tls-key-path",
			tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"site",
			"create",
			"--short-name",
			"test",
			"--full-title",
			"Test Site",
			"--template-name",
			"default",
		],
		{ cwd: tempRoot, env },
	);

	const siteId = parseCreatedId(siteResult.stdout, "site");
	const port = await reservePort();
	const baseUrl = `https://127.0.0.1:${port}`;
	const serverLogs = { stdout: "", stderr: "" };
	const server = spawn(
		websitesBinaryPath,
		[
			"--database-url",
			dbPath,
			"--tls-cert-path",
			tlsCertPath,
			"--tls-key-path",
			tlsKeyPath,
			"--frontend-url",
			baseUrl,
			...oidcTestArgs,
			"serve",
			"admin",
			"--listen",
			`127.0.0.1:${port}`,
		],
		{
			cwd: tempRoot,
			env,
			stdio: ["ignore", "pipe", "pipe"],
		},
	);
	server.stdout?.on("data", (data) => {
		serverLogs.stdout += data.toString();
	});
	server.stderr?.on("data", (data) => {
		serverLogs.stderr += data.toString();
	});
	await waitForPort(port, server, serverLogs, 30_000);

	return {
		tempRoot,
		uploadRoot,
		dbPath,
		databaseUrl,
		baseUrl,
		env,
		tlsCertPath,
		tlsKeyPath,
		port,
		server,
		serverLogs,
		siteId,
	};
}

export async function createUser(
	harness: TestHarness,
	subject: string,
	options: { admin?: boolean; email?: string } = {},
): Promise<string> {
	const { admin = false, email } = options;
	const result = await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"user",
			"create",
			"--subject",
			subject,
			...(email ? ["--email", email] : []),
			...(admin ? ["--admin"] : []),
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);
	return parseCreatedId(result.stdout, "user");
}

export async function createMembership(
	harness: TestHarness,
	userId: string,
	role: SiteRole,
	siteId = harness.siteId,
): Promise<string> {
	const result = await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"site",
			"member-add",
			"--site-id",
			siteId,
			"--user-id",
			userId,
			"--role",
			role,
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);
	return parseCreatedId(result.stdout, "membership");
}

export async function addMembership(
	harness: TestHarness,
	userId: string,
	role: SiteRole,
): Promise<void> {
	await createMembership(harness, userId, role);
}

export async function createTag(
	harness: TestHarness,
	name: string,
): Promise<string> {
	const result = await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"site",
			"tag-create",
			"--site-id",
			harness.siteId,
			"--name",
			name,
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);
	return parseCreatedId(result.stdout, "tag");
}

export async function createAssetWithThumbnail(
	harness: TestHarness,
	{
		originalFilename,
		storageBasename,
		thumbnailFilename,
	}: {
		originalFilename: string;
		storageBasename: string;
		thumbnailFilename: string;
	},
): Promise<string> {
	const createResult = await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"asset",
			"create",
			"--site-id",
			harness.siteId,
			"--uploader-sub",
			"test-user",
			"--original-filename",
			originalFilename,
			"--storage-basename",
			storageBasename,
			"--mime-type",
			"image/png",
			"--byte-length",
			"128",
			"--width",
			"800",
			"--height",
			"600",
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);
	const assetId = parseCreatedId(createResult.stdout, "asset");

	await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"asset",
			"variant-create",
			"--asset-id",
			assetId,
			"--variant-kind",
			"thumbnail",
			"--filename",
			thumbnailFilename,
			"--mime-type",
			"image/png",
			"--byte-length",
			"64",
			"--width",
			"320",
			"--height",
			"240",
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);

	return assetId;
}

export async function createContent(
	harness: TestHarness,
	{
		siteId,
		pageType,
		title,
		slug,
		pageContent,
		creatorSub,
		draft = true,
	}: CreateContentInput,
): Promise<string> {
	const result = await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"content",
			"create",
			"--site-id",
			siteId ?? harness.siteId,
			"--page-type",
			pageType,
			"--title",
			title,
			"--slug",
			slug,
			"--page-content",
			pageContent,
			"--creator-sub",
			creatorSub,
			...(draft ? ["--draft"] : []),
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);
	return parseCreatedId(result.stdout, "content");
}

export async function createSite(
	harness: TestHarness,
	{
		shortName,
		fullTitle,
		templateName = "default",
	}: {
		shortName: string;
		fullTitle: string;
		templateName?: string;
	},
): Promise<string> {
	const result = await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"site",
			"create",
			"--short-name",
			shortName,
			"--full-title",
			fullTitle,
			"--template-name",
			templateName,
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);
	return parseCreatedId(result.stdout, "site");
}

export async function createAlias(
	harness: TestHarness,
	{
		contentId,
		aliasPath,
		kind = "alias",
	}: {
		contentId: string;
		aliasPath: string;
		kind?: "primary" | "alias";
	},
): Promise<string> {
	const result = await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"content",
			"alias-create",
			"--content-id",
			contentId,
			"--site-id",
			harness.siteId,
			"--alias-path",
			aliasPath,
			"--kind",
			kind,
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);
	return parseCreatedId(result.stdout, "alias");
}

export async function updateContent(
	harness: TestHarness,
	{
		contentId,
		pageType,
		title,
		slug,
		pageContent,
		draft,
		publishedAt,
		editorSub,
	}: UpdateContentInput,
): Promise<void> {
	await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"content",
			"update",
			"--content-id",
			contentId,
			...(pageType ? ["--page-type", pageType] : []),
			...(title ? ["--title", title] : []),
			...(slug ? ["--slug", slug] : []),
			...(pageContent ? ["--page-content", pageContent] : []),
			...(draft === undefined ? [] : ["--draft", `${draft}`]),
			...(publishedAt ? ["--published-at", publishedAt] : []),
			"--editor-sub",
			editorSub,
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);
}

export async function listContentRevisions(
	harness: TestHarness,
	contentId: string,
): Promise<RevisionRow[]> {
	const result = await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"content",
			"revisions",
			"--content-id",
			contentId,
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);
	return parseRevisions(result.stdout);
}

export async function listAuditEvents(
	harness: TestHarness,
	siteId?: string,
): Promise<AuditEventRow[]> {
	const result = await runWebsitesCommand(
		[
			"--database-url",
			harness.dbPath,
			"--tls-cert-path",
			harness.tlsCertPath,
			"--tls-key-path",
			harness.tlsKeyPath,
			"--frontend-url",
			"https://127.0.0.1",
			...oidcTestArgs,
			"audit",
			"list",
			...(siteId ? ["--site-id", siteId] : []),
		],
		{ cwd: harness.tempRoot, env: harness.env },
	);
	return parseAuditEvents(result.stdout);
}

export async function seedSession(
	harness: TestHarness,
	subject: string,
	setAdmin = false,
): Promise<string> {
	const result = await runSessionSeedCommand(
		[
			"--database-url",
			harness.databaseUrl,
			"--user-sub",
			subject,
			...(setAdmin ? ["--set-admin"] : []),
		],
		{
			cwd: harness.tempRoot,
			env: harness.env,
		},
	);
	const sessionId = result.stdout.trim();
	if (!sessionId) {
		throw new Error("missing session id output");
	}
	return sessionId;
}

export async function createAuthenticatedPage(
	browser: Browser,
	harness: TestHarness,
	subject: string,
	setAdmin = false,
): Promise<AuthenticatedPage> {
	const sessionId = await seedSession(harness, subject, setAdmin);
	const context = await browser.newContext({ ignoreHTTPSErrors: true });
	await context.addCookies([
		{
			name: "id",
			value: sessionId,
			url: harness.baseUrl,
		},
	]);
	const page = await context.newPage();
	return { context, page, sessionId };
}

export async function createAuthenticatedApiContext(
	harness: TestHarness,
	subject: string,
	setAdmin = false,
): Promise<AuthenticatedApiContext> {
	const sessionId = await seedSession(harness, subject, setAdmin);
	const api = await playwrightRequest.newContext({
		baseURL: harness.baseUrl,
		ignoreHTTPSErrors: true,
		extraHTTPHeaders: {
			Cookie: `id=${sessionId}`,
		},
	});
	return { api, sessionId };
}

export async function seedAuthorizationFixtures(
	harness: TestHarness,
): Promise<AuthorizationFixtures> {
	const ownerSubject = "auth-owner";
	const ownerUserId = await createUser(harness, ownerSubject);
	const ownerMembershipId = await createMembership(
		harness,
		ownerUserId,
		"owner",
	);

	const contentId = await createContent(harness, {
		pageType: "page",
		title: "Authorization Page",
		slug: "authorization-page",
		pageContent: "Initial authorization body",
		creatorSub: ownerSubject,
	});
	await updateContent(harness, {
		contentId,
		title: "Authorization Page Updated",
		pageContent: "Updated authorization body",
		editorSub: ownerSubject,
	});
	const revisions = await listContentRevisions(harness, contentId);
	const secondRevision = revisions.find(
		(revision) => revision.revisionNumber === 2,
	);
	if (!secondRevision) {
		throw new Error("expected a second revision for auth fixtures");
	}

	const tagName = "auth-tag";
	const tagId = await createTag(harness, tagName);
	const assetOriginalFilename = "auth-image.png";
	const assetId = await createAssetWithThumbnail(harness, {
		originalFilename: assetOriginalFilename,
		storageBasename: "auth-image.png",
		thumbnailFilename: "auth-image_thumb.png",
	});

	const membershipTargetSubject = "auth-member-target";
	const membershipTargetUserId = await createUser(
		harness,
		membershipTargetSubject,
	);
	const membershipTargetId = await createMembership(
		harness,
		membershipTargetUserId,
		"viewer",
	);

	return {
		ownerSubject,
		ownerUserId,
		ownerMembershipId,
		contentId,
		revisionId: secondRevision.id,
		tagId,
		tagName,
		assetId,
		assetOriginalFilename,
		membershipTargetSubject,
		membershipTargetUserId,
		membershipTargetId,
		previewAssetPath: "style.css",
		siteFullTitle: "Test Site",
		templateName: "default",
	};
}

export async function cleanupHarness(harness: TestHarness): Promise<void> {
	if (!harness.server.killed) {
		harness.server.kill("SIGTERM");
	}
	await rm(harness.tempRoot, { recursive: true, force: true });
}
