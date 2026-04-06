import { expect, test, type APIResponse } from "@playwright/test";

import {
	cleanupHarness,
	createAuthenticatedApiContext,
	createMembership,
	createTag,
	createUser,
	extractCsrfToken,
	seedAuthorizationFixtures,
	setupHarness,
	tinyPngBytes,
	type AuthorizationFixtures,
	type SiteRole,
	type TestHarness,
} from "./support";
import { defaultTimeout } from "./global_setup";

type ScenarioName = "lowest" | "lower" | "no_membership" | "admin";

type RequestSetup = {
	path: string;
	form?: Record<string, string>;
	multipart?:
		| FormData
		| {
				[key: string]:
					| string
					| number
					| boolean
					| {
							/**
							 * File name
							 */
							name: string;

							/**
							 * File type
							 */
							mimeType: string;

							/**
							 * File content
							 */
							buffer: Buffer;
					  };
		  };
	beforeRequest?: () => Promise<void>;
	assertSuccess: (response: APIResponse) => Promise<void>;
};

type RouteCase = {
	name: string;
	method: "GET" | "POST";
	requiredRole: SiteRole;
	successStatus: number;
	lowerRole?: SiteRole;
	prepare: (
		harness: TestHarness,
		fixtures: AuthorizationFixtures,
		scenario: ScenarioName,
	) => Promise<RequestSetup>;
};

const unauthorizedNoMembership = (siteId: string) =>
	`missing membership for site ${siteId}`;

function roleSubject(role: SiteRole): string {
	return `auth-${role}-actor`;
}

async function ensureRoleActor(
	harness: TestHarness,
	cache: Map<SiteRole, string>,
	role: SiteRole,
): Promise<string> {
	const existing = cache.get(role);
	if (existing) {
		return existing;
	}

	const subject = roleSubject(role);
	const userId = await createUser(harness, subject);
	await createMembership(harness, userId, role);
	cache.set(role, subject);
	return subject;
}

async function runAs(
	harness: TestHarness,
	cache: Map<SiteRole, string>,
	role: SiteRole | undefined,
	scenario: ScenarioName,
): Promise<Awaited<ReturnType<typeof createAuthenticatedApiContext>>> {
	if (scenario === "admin") {
		return createAuthenticatedApiContext(
			harness,
			`auth-global-admin-${Date.now()}`,
			true,
		);
	}

	if (scenario === "no_membership") {
		const subject = `auth-no-membership-${Date.now()}`;
		await createUser(harness, subject);
		return createAuthenticatedApiContext(harness, subject);
	}

	if (!role) {
		throw new Error(`missing role for scenario ${scenario}`);
	}

	const subject = await ensureRoleActor(harness, cache, role);
	return createAuthenticatedApiContext(harness, subject);
}

async function makeMembershipTarget(
	harness: TestHarness,
	scenario: ScenarioName,
): Promise<{ subject: string; membershipId: string }> {
	const subject = `route-membership-${scenario}-${Date.now()}`;
	const userId = await createUser(harness, subject);
	const membershipId = await createMembership(harness, userId, "viewer");
	return { subject, membershipId };
}

async function makeTagTarget(
	harness: TestHarness,
	scenario: ScenarioName,
): Promise<{ tagName: string; tagId: string }> {
	const tagName = `route-tag-${scenario}-${Date.now()}`;
	const tagId = await createTag(harness, tagName);
	return { tagName, tagId };
}

async function requestWithSetup(
	api: Awaited<ReturnType<typeof createAuthenticatedApiContext>>["api"],
	route: RouteCase,
	setup: RequestSetup,
): Promise<APIResponse> {
	if (setup.beforeRequest) {
		await setup.beforeRequest();
	}
	return api.fetch(setup.path, {
		method: route.method,
		failOnStatusCode: false,
		maxRedirects: 0,
		...(setup.form ? { form: setup.form } : {}),
		...(setup.multipart ? { multipart: setup.multipart } : {}),
	});
}

const routeCases: RouteCase[] = [
	{
		name: "content preview",
		method: "GET",
		requiredRole: "viewer",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/content/${fixtures.contentId}/preview`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("Updated authorization body");
			},
		}),
	},
	{
		name: "preview asset",
		method: "GET",
		requiredRole: "viewer",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/preview-assets/${fixtures.previewAssetPath}`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("font-family");
			},
		}),
	},
	{
		name: "content detail",
		method: "GET",
		requiredRole: "viewer",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/content/${fixtures.contentId}`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("Content: /authorization-page");
			},
		}),
	},
	{
		name: "content advanced",
		method: "GET",
		requiredRole: "viewer",
		successStatus: 303,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/content/${fixtures.contentId}/advanced`,
			assertSuccess: async (response) => {
				expect(response.headers()["location"]).toBe(
					`/admin/site/${harness.siteId}/content/${fixtures.contentId}`,
				);
			},
		}),
	},
	{
		name: "content revisions",
		method: "GET",
		requiredRole: "viewer",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/content/${fixtures.contentId}/revisions`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("Revision Diffs");
			},
		}),
	},
	{
		name: "revision diff",
		method: "GET",
		requiredRole: "viewer",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/content/${fixtures.contentId}/revisions/${fixtures.revisionId}`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("Updated authorization body");
			},
		}),
	},
	{
		name: "assets list",
		method: "GET",
		requiredRole: "viewer",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/assets`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain(fixtures.assetOriginalFilename);
			},
		}),
	},
	{
		name: "tags list",
		method: "GET",
		requiredRole: "viewer",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/tags`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain(fixtures.tagName);
			},
		}),
	},
	{
		name: "content new",
		method: "GET",
		requiredRole: "author",
		lowerRole: "viewer",
		successStatus: 200,
		prepare: async (harness) => ({
			path: `/admin/site/${harness.siteId}/content/new`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("Create content");
			},
		}),
	},
	{
		name: "content create",
		method: "POST",
		requiredRole: "author",
		lowerRole: "viewer",
		successStatus: 303,
		prepare: async (harness, _fixtures, scenario) => {
			const suffix = `${scenario}-${Date.now()}`;
			return {
				path: `/admin/site/${harness.siteId}/content/new`,
				form: {
					page_type: "page",
					title: `Created ${suffix}`,
					slug: `created-${suffix}`,
					page_content: `Created body ${suffix}`,
					draft: "true",
					tag_list: "",
				},
				assertSuccess: async (response) => {
					const location = response.headers()["location"] ?? "";
					expect(location).toContain(`/admin/site/${harness.siteId}/content/`);
					expect(location).toContain("/edit");
				},
			};
		},
	},
	{
		name: "content editor",
		method: "GET",
		requiredRole: "author",
		lowerRole: "viewer",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/content/${fixtures.contentId}/edit`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("Save");
			},
		}),
	},
	{
		name: "content editor update",
		method: "POST",
		requiredRole: "author",
		lowerRole: "viewer",
		successStatus: 303,
		prepare: async (harness, fixtures, scenario) => ({
			path: `/admin/site/${harness.siteId}/content/${fixtures.contentId}/edit`,
			form: {
				page_type: "page",
				title: `Updated from ${scenario}`,
				slug: "authorization-page",
				page_content: `Updated body from ${scenario}`,
				draft: "true",
				published_at: "",
				tag_list: fixtures.tagName,
			},
			assertSuccess: async (response) => {
				expect(response.headers()["location"]).toBe(
					`/admin/site/${harness.siteId}/content/${fixtures.contentId}/edit?saved=1`,
				);
			},
		}),
	},
	{
		name: "asset library",
		method: "GET",
		requiredRole: "viewer",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/api/site/${harness.siteId}/assets/library`,
			assertSuccess: async (response) => {
				const payload = await response.json();
				expect(Array.isArray(payload.assets)).toBe(true);
				expect(
					payload.assets.some(
						(asset: { original_filename: string }) =>
							asset.original_filename === fixtures.assetOriginalFilename,
					),
				).toBe(true);
			},
		}),
	},
	{
		name: "asset upload form",
		method: "GET",
		requiredRole: "author",
		lowerRole: "viewer",
		successStatus: 200,
		prepare: async (harness) => ({
			path: `/admin/site/${harness.siteId}/assets/new`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("Upload File");
			},
		}),
	},
	{
		name: "asset upload",
		method: "POST",
		requiredRole: "author",
		lowerRole: "viewer",
		successStatus: 303,
		prepare: async (harness, _fixtures, scenario) => ({
			path: `/admin/site/${harness.siteId}/assets/new`,
			multipart: {
				file: {
					name: `upload-${scenario}.png`,
					mimeType: "image/png",
					buffer: tinyPngBytes,
				},
			},
			assertSuccess: async (response) => {
				expect(response.headers()["location"]).toBe(
					`/admin/site/${harness.siteId}/assets`,
				);
			},
		}),
	},
	{
		name: "asset replace form",
		method: "GET",
		requiredRole: "author",
		lowerRole: "viewer",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/assets/${fixtures.assetId}/replace`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("Replace the uploaded asset");
			},
		}),
	},
	{
		name: "asset replace",
		method: "POST",
		requiredRole: "author",
		lowerRole: "viewer",
		successStatus: 303,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/assets/${fixtures.assetId}/replace`,
			multipart: {
				file: {
					name: fixtures.assetOriginalFilename,
					mimeType: "image/png",
					buffer: tinyPngBytes,
				},
			},
			assertSuccess: async (response) => {
				expect(response.headers()["location"]).toBe(
					`/admin/site/${harness.siteId}/assets`,
				);
			},
		}),
	},
	{
		name: "render route",
		method: "GET",
		requiredRole: "editor",
		lowerRole: "author",
		successStatus: 200,
		prepare: async (harness) => ({
			path: `/admin/site/${harness.siteId}/render`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("Site rendered with");
			},
		}),
	},
	{
		name: "tag create",
		method: "POST",
		requiredRole: "editor",
		lowerRole: "author",
		successStatus: 303,
		prepare: async (harness, _fixtures, scenario) => ({
			path: `/admin/site/${harness.siteId}/tags/new`,
			form: {
				name: `created-tag-${scenario}-${Date.now()}`,
			},
			assertSuccess: async (response) => {
				expect(response.headers()["location"]).toBe(
					`/admin/site/${harness.siteId}/tags`,
				);
			},
		}),
	},
	{
		name: "tag delete",
		method: "POST",
		requiredRole: "editor",
		lowerRole: "author",
		successStatus: 303,
		prepare: async (harness, _fixtures, scenario) => {
			const { tagId } = await makeTagTarget(harness, scenario);
			return {
				path: `/admin/site/${harness.siteId}/tags/${tagId}/delete`,
				assertSuccess: async (response) => {
					expect(response.headers()["location"]).toBe(
						`/admin/site/${harness.siteId}/tags`,
					);
				},
			};
		},
	},
	{
		name: "memberships list",
		method: "GET",
		requiredRole: "owner",
		lowerRole: "editor",
		successStatus: 200,
		prepare: async (harness, fixtures) => ({
			path: `/admin/site/${harness.siteId}/memberships`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain(
					fixtures.membershipTargetSubject,
				);
			},
		}),
	},
	{
		name: "membership create",
		method: "POST",
		requiredRole: "owner",
		lowerRole: "editor",
		successStatus: 303,
		prepare: async (harness, _fixtures, scenario) => {
			const subject = `created-member-${scenario}-${Date.now()}`;
			await createUser(harness, subject);
			return {
				path: `/admin/site/${harness.siteId}/memberships/new`,
				form: {
					subject,
					role: "viewer",
				},
				assertSuccess: async (response) => {
					expect(response.headers()["location"]).toBe(
						`/admin/site/${harness.siteId}/memberships`,
					);
				},
			};
		},
	},
	{
		name: "membership update",
		method: "POST",
		requiredRole: "owner",
		lowerRole: "editor",
		successStatus: 303,
		prepare: async (harness, _fixtures, scenario) => {
			const { membershipId } = await makeMembershipTarget(harness, scenario);
			return {
				path: `/admin/site/${harness.siteId}/memberships/${membershipId}/update`,
				form: {
					role: "author",
				},
				assertSuccess: async (response) => {
					expect(response.headers()["location"]).toBe(
						`/admin/site/${harness.siteId}/memberships`,
					);
				},
			};
		},
	},
	{
		name: "membership remove",
		method: "POST",
		requiredRole: "owner",
		lowerRole: "editor",
		successStatus: 303,
		prepare: async (harness, _fixtures, scenario) => {
			const { membershipId } = await makeMembershipTarget(harness, scenario);
			return {
				path: `/admin/site/${harness.siteId}/memberships/${membershipId}/remove`,
				assertSuccess: async (response) => {
					expect(response.headers()["location"]).toBe(
						`/admin/site/${harness.siteId}/memberships`,
					);
				},
			};
		},
	},
	{
		name: "site settings update",
		method: "POST",
		requiredRole: "owner",
		lowerRole: "editor",
		successStatus: 303,
		prepare: async (harness, fixtures, scenario) => ({
			path: `/admin/site/${harness.siteId}/settings`,
			form: {
				full_title: `Test Site ${scenario} ${Date.now()}`,
				template_name: fixtures.templateName,
			},
			assertSuccess: async (response) => {
				expect(response.headers()["location"]).toBe(
					`/admin/site/${harness.siteId}/settings`,
				);
			},
		}),
	},
	{
		name: "template override editor",
		method: "GET",
		requiredRole: "owner",
		lowerRole: "editor",
		successStatus: 200,
		prepare: async (harness) => ({
			path: `/admin/site/${harness.siteId}/settings/templates/page.html`,
			assertSuccess: async (response) => {
				expect(await response.text()).toContain("Template Override: page.html");
			},
		}),
	},
	{
		name: "template override update",
		method: "POST",
		requiredRole: "owner",
		lowerRole: "editor",
		successStatus: 303,
		prepare: async (harness, _fixtures, scenario) => ({
			path: `/admin/site/${harness.siteId}/settings/templates/page.html`,
			form: {
				source: `{% extends "base_template.html" %}{% block content %}<article>${scenario}</article>{% endblock %}`,
			},
			assertSuccess: async (response) => {
				expect(response.headers()["location"]).toBe(
					`/admin/site/${harness.siteId}/settings/templates/page.html?saved=1`,
				);
			},
		}),
	},
	{
		name: "template override reset",
		method: "POST",
		requiredRole: "owner",
		lowerRole: "editor",
		successStatus: 303,
		prepare: async (harness, fixtures, scenario) => ({
			path: `/admin/site/${harness.siteId}/settings/templates/page.html/reset`,
			form: {},
			assertSuccess: async (response) => {
				expect(response.headers()["location"]).toBe(
					`/admin/site/${harness.siteId}/settings/templates/page.html?reset=1`,
				);
			},
			beforeRequest: async () => {
				const ownerContext = await createAuthenticatedApiContext(
					harness,
					fixtures.ownerSubject,
				);
				try {
					const seedResponse = await ownerContext.api.fetch(
						`/admin/site/${harness.siteId}/settings/templates/page.html`,
						{
							method: "POST",
							maxRedirects: 0,
							failOnStatusCode: false,
							form: {
								source: `{% extends "base_template.html" %}{% block content %}<article>${scenario}</article>{% endblock %}`,
							},
						},
					);
					expect(seedResponse.status()).toBe(303);
				} finally {
					await ownerContext.api.dispose();
				}
			},
		}),
	},
];

test.describe("admin authorization coverage", () => {
	test.setTimeout(defaultTimeout);

	for (const route of routeCases) {
		test(`${route.method} ${route.name}`, async () => {
			const harness = await setupHarness();

			try {
				const fixtures = await seedAuthorizationFixtures(harness);
				const actorCache = new Map<SiteRole, string>();
				actorCache.set("owner", fixtures.ownerSubject);

				const lowestContext = await runAs(
					harness,
					actorCache,
					route.requiredRole,
					"lowest",
				);
				try {
					const setup = await route.prepare(harness, fixtures, "lowest");
					const response = await requestWithSetup(
						lowestContext.api,
						route,
						setup,
					);
					expect(response.status()).toBe(route.successStatus);
					await setup.assertSuccess(response);
				} finally {
					await lowestContext.api.dispose();
				}

				if (route.lowerRole) {
					const lowerContext = await runAs(
						harness,
						actorCache,
						route.lowerRole,
						"lower",
					);
					try {
						const setup = await route.prepare(harness, fixtures, "lower");
						const response = await requestWithSetup(
							lowerContext.api,
							route,
							setup,
						);
						expect(response.status()).toBe(401);
						expect(await response.text()).toContain(
							"does not satisfy required role",
						);
					} finally {
						await lowerContext.api.dispose();
					}
				}

				const noMembershipContext = await runAs(
					harness,
					actorCache,
					undefined,
					"no_membership",
				);
				try {
					const setup = await route.prepare(harness, fixtures, "no_membership");
					const response = await requestWithSetup(
						noMembershipContext.api,
						route,
						setup,
					);
					expect(response.status()).toBe(401);
					expect(await response.text()).toContain(
						unauthorizedNoMembership(harness.siteId),
					);
				} finally {
					await noMembershipContext.api.dispose();
				}

				const adminContext = await runAs(
					harness,
					actorCache,
					undefined,
					"admin",
				);
				try {
					const setup = await route.prepare(harness, fixtures, "admin");
					const response = await requestWithSetup(
						adminContext.api,
						route,
						setup,
					);
					expect(response.status()).toBe(route.successStatus);
					await setup.assertSuccess(response);
				} finally {
					await adminContext.api.dispose();
				}
			} finally {
				await cleanupHarness(harness);
			}
		});
	}
});

test("site delete confirmation and deletion require a global admin session", async () => {
	const harness = await setupHarness();

	try {
		const ownerId = await createUser(harness, "delete-site-owner");
		await createMembership(harness, ownerId, "owner");

		const ownerContext = await createAuthenticatedApiContext(
			harness,
			"delete-site-owner",
		);
		try {
			const confirmResponse = await ownerContext.api.fetch(
				`/admin/site/${harness.siteId}/delete`,
				{
					method: "GET",
					failOnStatusCode: false,
					maxRedirects: 0,
				},
			);
			expect(confirmResponse.status()).toBe(401);
			expect(await confirmResponse.text()).toContain(
				"global admin access is required",
			);

			const response = await ownerContext.api.fetch(
				`/admin/site/${harness.siteId}/delete`,
				{
					method: "POST",
					form: {
						csrf_token: "invalid",
					},
					failOnStatusCode: false,
					maxRedirects: 0,
				},
			);
			expect(response.status()).toBe(401);
			expect(await response.text()).toContain(
				"global admin access is required",
			);
		} finally {
			await ownerContext.api.dispose();
		}

		const adminContext = await createAuthenticatedApiContext(
			harness,
			`delete-site-admin-${Date.now()}`,
			true,
		);
		try {
			const confirmResponse = await adminContext.api.fetch(
				`/admin/site/${harness.siteId}/delete`,
				{
					method: "GET",
					failOnStatusCode: false,
					maxRedirects: 0,
				},
			);
			expect(confirmResponse.status()).toBe(200);
			const confirmBody = await confirmResponse.text();
			expect(confirmBody).toContain("Confirm Site Deletion");
			const csrfToken = extractCsrfToken(confirmBody);

			const missingTokenResponse = await adminContext.api.fetch(
				`/admin/site/${harness.siteId}/delete`,
				{
					method: "POST",
					form: {
						csrf_token: "",
					},
					failOnStatusCode: false,
					maxRedirects: 0,
				},
			);
			expect(missingTokenResponse.status()).toBe(400);
			expect(await missingTokenResponse.text()).toContain("csrf token");

			const response = await adminContext.api.fetch(
				`/admin/site/${harness.siteId}/delete`,
				{
					method: "POST",
					form: {
						csrf_token: csrfToken,
					},
					failOnStatusCode: false,
					maxRedirects: 0,
				},
			);
			expect(response.status()).toBe(303);
			expect(response.headers()["location"]).toBe("/admin?deleted=1");
		} finally {
			await adminContext.api.dispose();
		}
	} finally {
		await cleanupHarness(harness);
	}
});
