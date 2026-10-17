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

test("settings search finds an individual setting and jumps to its tab", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await expect(dialog).toBeVisible();

  await dialog
    .getByRole("searchbox", { name: "Search settings" })
    .fill("alt download");
  await dialog.getByRole("button", { name: /Alt download limit/ }).click();
  await expect(
    dialog.getByText("Alternative Speed Limits", { exact: true }),
  ).toBeVisible();
});

test("settings search focuses the matched field, not just its tab", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog
    .getByRole("searchbox", { name: "Search settings" })
    .fill("peer limit");
  await dialog.getByRole("button", { name: /Peer limit/ }).click();
  await expect(dialog.locator('input[name="peer_limit"]')).toBeFocused();
});

test("settings search marks restart-required settings", async ({ page }) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog
    .getByRole("searchbox", { name: "Search settings" })
    .fill("listen port");
  await expect(
    dialog.getByRole("button", { name: /Listen port.*Restart required/ }),
  ).toBeVisible();
});

test("the shortcut dialog is named by its title", async ({ page }) => {
  await page.keyboard.press("?");
  await expect(
    page.getByRole("dialog", { name: "Keyboard shortcuts" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
});

test("a pause is announced in the live region", async ({ page }) => {
  await page.getByRole("button", { name: /^Downloading/ }).click();
  await page.locator("tbody tr").first().click();
  // A focused row claims Space for its own selection, so move focus off it
  await page.evaluate(() =>
    (document.activeElement as HTMLElement | null)?.blur(),
  );
  await page.keyboard.press("Space");
  await expect(page.getByRole("status")).toContainText("Paused");
});

test("the detail pane keeps its tab across a reload", async ({ page }) => {
  await page.locator("tbody tr").first().click();
  await page
    .locator("button", { hasText: /^Peers$/ })
    .last()
    .click();
  await page.reload();
  await expect(page.locator("tbody tr").first()).toBeVisible();
  await page.locator("tbody tr").first().click();
  await expect(
    page.locator("button", { hasText: /^Peers$/ }).last(),
  ).toHaveClass(/border-primary/);
});
