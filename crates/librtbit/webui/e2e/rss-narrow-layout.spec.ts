import { expect, test } from "@playwright/test";

// UI fix WP-F: below the `sm` breakpoint the RSS tables become stacked rows.

async function openRss(page: import("@playwright/test").Page) {
  await page.goto("/mock.html");
  const sidebarToggle = page.getByTitle("Open sidebar");
  if (await sidebarToggle.isVisible()) {
    await sidebarToggle.click();
    await page
      .getByRole("dialog", { name: "Navigation" })
      .getByRole("button", { name: "RSS" })
      .click();
  } else {
    await page.getByRole("button", { name: "RSS" }).click();
  }
}

test("RSS lists render as stacked rows at 390px", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await openRss(page);

  const items = page.getByRole("list", { name: "RSS items" });
  await expect(items).toBeVisible();
  await expect(items.getByRole("listitem").first()).toBeVisible();
  await expect(page.locator("table")).toBeHidden();

  await page.getByRole("button", { name: /^Feeds/ }).click();
  const feeds = page.getByRole("list", { name: "RSS feeds" });
  await expect(feeds).toBeVisible();
  await expect(feeds.getByText(/Active|Disabled/).first()).toBeVisible();
  await expect(
    feeds.getByRole("button", { name: /^Edit feed / }).first(),
  ).toBeVisible();

  await page.getByRole("button", { name: /^Rules/ }).click();
  await expect(page.getByRole("list", { name: "RSS rules" })).toBeVisible();
});

test("RSS keeps the table layout on wide screens", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openRss(page);

  await expect(page.locator("table")).toBeVisible();
  await expect(page.getByRole("list", { name: "RSS items" })).toBeHidden();
});

test("RSS feed dialog title is a level-2 heading", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openRss(page);
  await page.getByRole("button", { name: /^Feeds/ }).click();
  await page.getByRole("button", { name: "Add Feed" }).click();

  await expect(
    page.getByRole("heading", { level: 2, name: "Add Feed" }),
  ).toBeVisible();
});
