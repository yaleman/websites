import { readFile } from "node:fs/promises";
import path from "node:path";

import { expect, test } from "@playwright/test";

import {
  addMembership,
  cleanupHarness,
  createAuthenticatedPage,
  createContent,
  createUser,
  setupHarness,
} from "./support";

const overrideSource = `{% extends "base_template.html" %}
{% block content %}
<article data-site-template="override">{{ title }}</article>
<div>{{ content }}</div>
{% endblock %}`;

test.describe("template overrides", () => {
  test.setTimeout(120_000);

  test("saves and resets a per-site page template override", async ({ browser }) => {
    const harness = await setupHarness();

    try {
      const subject = "template-owner";
      const userId = await createUser(harness, subject);
      await addMembership(harness, userId, "owner");

      const contentId = await createContent(harness, {
        pageType: "page",
        title: "Override Page",
        slug: "override-page",
        pageContent: "Override body",
        creatorSub: subject,
        draft: false,
      });

      const { context, page } = await createAuthenticatedPage(
        browser,
        harness,
        subject,
      );

      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings`,
        { waitUntil: "domcontentloaded" },
      );
      await expect(page.getByRole("heading", { name: "Template Overrides" })).toBeVisible();
      await expect(page.locator("body")).toContainText("page.html");

      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings/templates/page.html`,
        { waitUntil: "domcontentloaded" },
      );
      await expect(page.getByRole("heading", { name: "Template Override: page.html" })).toBeVisible();
      await page.getByLabel("Template Source").fill(overrideSource);
      await page.getByRole("button", { name: "Save override" }).click();
      await expect(page.locator("[data-clear-query-param='saved']")).toContainText(
        "Template override saved.",
      );

      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/preview`,
        { waitUntil: "domcontentloaded" },
      );
      expect(await page.content()).toContain('data-site-template="override"');

      const renderResponse = await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/render`,
        { waitUntil: "domcontentloaded" },
      );
      expect(renderResponse?.status()).toBe(200);
      await expect(page.locator("body")).toContainText("Site rendered with");
      const renderedPath = path.join(
        harness.tempRoot,
        "rendered",
        "test",
        "override-page",
        "index.html",
      );
      await expect
        .poll(async () => {
          try {
            return await readFile(renderedPath, "utf8");
          } catch {
            return "";
          }
        })
        .toContain('data-site-template="override"');

      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/settings/templates/page.html`,
        { waitUntil: "domcontentloaded" },
      );
      await page.getByRole("button", { name: "Reset override" }).click();
      await expect(page.locator("[data-clear-query-param='reset']")).toContainText(
        "Template override reset.",
      );

      await page.goto(
        `https://127.0.0.1:${harness.port}/admin/site/${harness.siteId}/content/${contentId}/preview`,
        { waitUntil: "domcontentloaded" },
      );
      expect(await page.content()).not.toContain('data-site-template="override"');

      await context.close();
    } finally {
      await cleanupHarness(harness);
    }
  });
});
