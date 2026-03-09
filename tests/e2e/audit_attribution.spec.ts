import { expect, test } from "@playwright/test";

import {
  cleanupHarness,
  createAuthenticatedApiContext,
  createContent,
  createMembership,
  createUser,
  listAuditEvents,
  listContentRevisions,
  setupHarness,
  tinyPngBytes,
  type AuditEventRow,
  type SiteRole,
  type TestHarness,
} from "./support";

function parseContentIdFromLocation(location: string | undefined, siteId: string): string {
  const match = location?.match(new RegExp(`/admin/site/${siteId}/content/([^/?]+)`));
  if (!match) {
    throw new Error(`failed to parse content id from location: ${location ?? "(missing)"}`);
  }
  return match[1];
}

function findAuditEvent(
  events: AuditEventRow[],
  criteria: {
    actorSub: string;
    eventType: string;
    entityId?: string;
    entityType?: string;
  },
): AuditEventRow {
  const match = events.find((event) => (
    event.actorSub === criteria.actorSub
    && event.eventType === criteria.eventType
    && (criteria.entityId === undefined || event.entityId === criteria.entityId)
    && (criteria.entityType === undefined || event.entityType === criteria.entityType)
  ));
  if (!match) {
    throw new Error(
      `missing audit event ${criteria.eventType} for ${criteria.actorSub} (${criteria.entityId ?? "any"})`,
    );
  }
  return match;
}

async function createRoleApi(
  harness: TestHarness,
  subject: string,
  role: SiteRole,
) {
  const userId = await createUser(harness, subject);
  await createMembership(harness, userId, role);
  return createAuthenticatedApiContext(harness, subject);
}

async function assertRevisionEditorSub(
  harness: TestHarness,
  subject: string,
  contentId: string,
  revisionId: string,
): Promise<void> {
  const context = await createAuthenticatedApiContext(harness, subject);
  try {
    const response = await context.api.get(
      `/admin/site/${harness.siteId}/content/${contentId}/revisions/${revisionId}`,
      { failOnStatusCode: false },
    );
    expect(response.status()).toBe(200);
    const body = await response.text();
    expect(body).toContain("editor_sub");
    expect(body).toContain(subject);
  } finally {
    await context.api.dispose();
  }
}

test.describe("web audit attribution", () => {
  test.setTimeout(120_000);

  test("content creation records the acting user in audit and revision history", async () => {
    const harness = await setupHarness();
    const actor = "audit-create-author";

    try {
      const context = await createRoleApi(harness, actor, "author");
      try {
        const response = await context.api.fetch(
          `/admin/site/${harness.siteId}/content/new`,
          {
            method: "POST",
            form: {
              page_type: "page",
              title: "Audit Content",
              slug: "audit-content",
              page_content: "Audit body",
              draft: "true",
              tag_list: "",
            },
            failOnStatusCode: false,
            maxRedirects: 0,
          },
        );
        expect(response.status()).toBe(303);
        const contentId = parseContentIdFromLocation(
          response.headers()["location"],
          harness.siteId,
        );

        const events = await listAuditEvents(harness, harness.siteId);
        const event = findAuditEvent(events, {
          actorSub: actor,
          eventType: "create_content",
          entityId: contentId,
          entityType: "content_item",
        });
        expect(event.siteId).toBe(harness.siteId);

        const revisions = await listContentRevisions(harness, contentId);
        expect(revisions).toHaveLength(1);
        await assertRevisionEditorSub(harness, actor, contentId, revisions[0].id);
      } finally {
        await context.api.dispose();
      }
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("content updates record the acting user in audit and revision history", async () => {
    const harness = await setupHarness();
    const creator = "original-content-creator";
    const actor = "audit-update-author";

    try {
      const creatorId = await createUser(harness, creator);
      await createMembership(harness, creatorId, "owner");
      const contentId = await createContent(harness, {
        pageType: "page",
        title: "Original Title",
        slug: "original-title",
        pageContent: "Original body",
        creatorSub: creator,
      });

      const context = await createRoleApi(harness, actor, "author");
      try {
        const response = await context.api.fetch(
          `/admin/site/${harness.siteId}/content/${contentId}/edit`,
          {
            method: "POST",
            form: {
              page_type: "page",
              title: "Updated Title",
              slug: "original-title",
              page_content: "Updated from the web editor",
              draft: "true",
              published_at: "",
              tag_list: "",
            },
            failOnStatusCode: false,
            maxRedirects: 0,
          },
        );
        expect(response.status()).toBe(303);
        expect(response.headers()["location"]).toBe(
          `/admin/site/${harness.siteId}/content/${contentId}/edit?saved=1`,
        );

        const events = await listAuditEvents(harness, harness.siteId);
        findAuditEvent(events, {
          actorSub: actor,
          eventType: "update_content",
          entityId: contentId,
          entityType: "content_item",
        });

        const revisions = await listContentRevisions(harness, contentId);
        expect(revisions[0].revisionNumber).toBe(2);
        await assertRevisionEditorSub(harness, actor, contentId, revisions[0].id);
      } finally {
        await context.api.dispose();
      }
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("membership changes record the acting user in audit events", async () => {
    const harness = await setupHarness();
    const actor = "membership-owner";

    try {
      const context = await createRoleApi(harness, actor, "owner");
      try {
        const targetSubject = "membership-target";
        await createUser(harness, targetSubject);
        const createResponse = await context.api.fetch(
          `/admin/site/${harness.siteId}/memberships/new`,
          {
            method: "POST",
            form: {
              subject: targetSubject,
              role: "viewer",
            },
            failOnStatusCode: false,
            maxRedirects: 0,
          },
        );
        expect(createResponse.status()).toBe(303);

        const created = findAuditEvent(await listAuditEvents(harness, harness.siteId), {
          actorSub: actor,
          eventType: "create_membership",
          entityType: "site_membership",
        });

        const updateResponse = await context.api.fetch(
          `/admin/site/${harness.siteId}/memberships/${created.entityId}/update`,
          {
            method: "POST",
            form: {
              role: "author",
            },
            failOnStatusCode: false,
            maxRedirects: 0,
          },
        );
        expect(updateResponse.status()).toBe(303);
        findAuditEvent(await listAuditEvents(harness, harness.siteId), {
          actorSub: actor,
          eventType: "update_membership",
          entityId: created.entityId,
          entityType: "site_membership",
        });

        const removeResponse = await context.api.fetch(
          `/admin/site/${harness.siteId}/memberships/${created.entityId}/remove`,
          {
            method: "POST",
            failOnStatusCode: false,
            maxRedirects: 0,
          },
        );
        expect(removeResponse.status()).toBe(303);
        findAuditEvent(await listAuditEvents(harness, harness.siteId), {
          actorSub: actor,
          eventType: "delete_membership",
          entityId: created.entityId,
          entityType: "site_membership",
        });
      } finally {
        await context.api.dispose();
      }
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("tag changes record the acting user in audit events", async () => {
    const harness = await setupHarness();
    const actor = "tag-editor-actor";

    try {
      const context = await createRoleApi(harness, actor, "editor");
      try {
        const createResponse = await context.api.fetch(
          `/admin/site/${harness.siteId}/tags/new`,
          {
            method: "POST",
            form: {
              name: "audit-tag",
            },
            failOnStatusCode: false,
            maxRedirects: 0,
          },
        );
        expect(createResponse.status()).toBe(303);

        const created = findAuditEvent(await listAuditEvents(harness, harness.siteId), {
          actorSub: actor,
          eventType: "create_tag",
          entityType: "tag",
        });

        const deleteResponse = await context.api.fetch(
          `/admin/site/${harness.siteId}/tags/${created.entityId}/delete`,
          {
            method: "POST",
            failOnStatusCode: false,
            maxRedirects: 0,
          },
        );
        expect(deleteResponse.status()).toBe(303);
        findAuditEvent(await listAuditEvents(harness, harness.siteId), {
          actorSub: actor,
          eventType: "delete_tag",
          entityId: created.entityId,
          entityType: "tag",
        });
      } finally {
        await context.api.dispose();
      }
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("asset uploads record the acting user in audit events", async () => {
    const harness = await setupHarness();
    const actor = "asset-author-actor";

    try {
      const context = await createRoleApi(harness, actor, "author");
      try {
        const response = await context.api.fetch(
          `/admin/site/${harness.siteId}/assets/new`,
          {
            method: "POST",
            multipart: {
              file: {
                name: "audit-upload.png",
                mimeType: "image/png",
                buffer: tinyPngBytes,
              },
            },
            failOnStatusCode: false,
            maxRedirects: 0,
          },
        );
        expect(response.status()).toBe(303);

        findAuditEvent(await listAuditEvents(harness, harness.siteId), {
          actorSub: actor,
          eventType: "create_asset",
          entityType: "asset",
        });
      } finally {
        await context.api.dispose();
      }
    } finally {
      await cleanupHarness(harness);
    }
  });

  test("site settings updates record the acting user in audit events", async () => {
    const harness = await setupHarness();
    const actor = "settings-owner-actor";

    try {
      const context = await createRoleApi(harness, actor, "owner");
      try {
        const response = await context.api.fetch(
          `/admin/site/${harness.siteId}/settings`,
          {
            method: "POST",
            form: {
              full_title: "Audit Settings Title",
              template_name: "default",
            },
            failOnStatusCode: false,
            maxRedirects: 0,
          },
        );
        expect(response.status()).toBe(303);
        findAuditEvent(await listAuditEvents(harness, harness.siteId), {
          actorSub: actor,
          eventType: "update_site",
          entityId: harness.siteId,
          entityType: "site",
        });
      } finally {
        await context.api.dispose();
      }
    } finally {
      await cleanupHarness(harness);
    }
  });
});
