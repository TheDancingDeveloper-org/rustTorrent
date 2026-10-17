import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/mock.html");
  await expect(page.locator("tbody tr").first()).toBeVisible();
});

test("? opens the shortcut reference and Escape closes it", async ({
  page,
}) => {
  await page.keyboard.press("?");
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Keyboard shortcuts");
  await expect(dialog).toContainText("Pause or resume");

  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});

test("Backspace no longer opens the delete confirmation", async ({ page }) => {
  await page.locator("tbody tr").first().click();
  await page.keyboard.press("Backspace");
  await expect(page.getByRole("dialog")).toHaveCount(0);

  await page.keyboard.press("Delete");
  await expect(page.getByRole("dialog")).toBeVisible();
});

test("settings search narrows the tabs and reports no matches", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const search = page.getByRole("searchbox", { name: "Search settings" });
  await search.fill("dht");
  const tabs = page.getByRole("dialog").getByRole("button");
  await expect(tabs.filter({ hasText: /^DHT$/ })).toBeVisible();
  await expect(tabs.filter({ hasText: /^Speed$/ })).toHaveCount(0);

  await search.fill("zzzz-no-such-setting");
  await expect(page.getByText("No settings match")).toBeVisible();
});

test("sort column survives a reload", async ({ page }) => {
  await page.getByRole("columnheader", { name: /Name/ }).click();
  await page.reload();
  await expect(page.locator("tbody tr").first()).toBeVisible();
  await expect(
    page.getByRole("columnheader", { name: /Name/ }),
  ).toHaveAttribute("aria-sort", /ascending|descending/);
});
