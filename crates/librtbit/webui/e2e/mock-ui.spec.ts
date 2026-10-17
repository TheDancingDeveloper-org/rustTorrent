import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/mock.html");
  await expect(page).toHaveTitle(/rustTorrent/i);
  await expect(page.locator("tbody tr").first()).toBeVisible();
});

test("renders fictional release data and live session stats", async ({
  page,
}) => {
  await expect(page.getByTitle("RustTorrent Demo")).toBeVisible();
  await expect(page.locator("tbody tr")).toHaveCount(12);
  await expect(
    page.getByRole("button", { name: /^All/ }).first(),
  ).toContainText("36");
  await expect(page.locator("tbody tr").first()).toContainText(
    "Telescope.Raw.Sample.Data",
  );
  await expect(page.getByTitle("Toggle dark mode")).toBeVisible();
});

test("filters torrents by search and status", async ({ page }) => {
  const search = page.locator("[data-search-input]");
  await search.fill("NebulaOS");
  await expect(page.locator("tbody tr")).not.toHaveCount(1000);
  await expect(page.locator("tbody tr").first()).toContainText("NebulaOS");

  await search.fill("");
  await page.getByRole("button", { name: /^Paused/ }).click();
  await expect(page.locator("tbody tr")).toHaveCount(13);
  await expect(page.getByRole("button", { name: /^Paused/ })).toHaveClass(
    /bg-primary/,
  );
});

test("shows simulated RSS release history", async ({ page }) => {
  await page.getByRole("button", { name: "RSS" }).click();
  await expect(
    page.locator("tbody").getByText("Open Media Weekly").first(),
  ).toBeVisible();
  await expect(page.getByText("Glass River", { exact: false })).toBeVisible();
  await expect(page.getByText("Done").first()).toBeVisible();
  await expect(page.getByText("Pending").first()).toBeVisible();
});

test("selects a torrent and exposes safe bulk actions", async ({ page }) => {
  // Select from the downloading filter so pausing must cause a real state
  // transition (the default first row is intentionally already paused).
  await page
    .getByRole("button", { name: /^Downloading/ })
    .first()
    .click();
  const firstRow = page.locator("tbody tr").first();
  await firstRow.locator('input[type="checkbox"]').click();
  await expect(page.getByTitle("Pause selected")).toBeEnabled();
  await expect(page.getByTitle("Resume selected")).toBeEnabled();
  await expect(page.getByTitle("Delete selected")).toBeEnabled();
  const pausedButton = page.getByRole("button", { name: /^Paused/ }).first();
  const pausedBefore = Number(
    (await pausedButton.textContent())?.match(/\d+/)?.[0],
  );
  await page.getByTitle("Pause selected").click();
  await expect
    .poll(async () => {
      const text = await pausedButton.textContent();
      return Number(text?.match(/\d+/)?.[0]);
    })
    .toBe(pausedBefore + 1);
});

test("starts an integrity check from the torrent context menu", async ({
  page,
}) => {
  const firstRow = page.locator("tbody tr").first();
  await firstRow.click({ button: "right" });
  const recheck = page.getByRole("menuitem", { name: "Force Recheck" });
  await expect(recheck).toBeVisible();
  await recheck.click();
  await expect(firstRow).toContainText("Checking");
});

test("disables recheck while a torrent is already checking", async ({
  page,
}) => {
  const firstRow = page.locator("tbody tr").first();
  const torrentName = await firstRow
    .locator("td div[title]")
    .first()
    .getAttribute("title");
  expect(torrentName).toBeTruthy();
  await firstRow.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Force Recheck" }).click();
  const checkingRow = page.locator("tbody tr").filter({
    has: page.getByTitle(torrentName!, { exact: true }),
  });
  await expect(checkingRow).toContainText("Checking");
  await checkingRow.click({ button: "right" });
  await expect(
    page.getByRole("menuitem", { name: "Force Recheck" }),
  ).toBeDisabled();
});

test("supports configuration and responsive card layout", async ({ page }) => {
  await page.getByTitle("Configure").click();
  await expect(
    page.getByText(/General|Connection|Folders/).first(),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  await page.setViewportSize({ width: 390, height: 844 });
  await page.reload();
  await expect(page.getByTitle("Open sidebar")).toBeVisible();
  await expect(page.locator("tbody tr")).toHaveCount(0);
  await expect(
    page.locator("span").getByText("Paused", { exact: true }).first(),
  ).toBeVisible();
});

test("selects and opens the menu for torrents with the keyboard", async ({
  page,
}) => {
  const rows = page.locator('[role="row"][data-row-index]');
  await rows.nth(0).focus();

  await page.keyboard.press("ArrowDown");
  await expect(rows.nth(1)).toBeFocused();
  await expect(rows.nth(1)).toHaveAttribute("aria-selected", "true");

  await page.keyboard.press(" ");
  await expect(rows.nth(1)).toHaveAttribute("aria-selected", "false");

  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Torrent actions" });
  await expect(menu).toBeVisible();
  await expect(
    menu.locator('[role^="menuitem"]:not(:disabled)').first(),
  ).toBeFocused();

  // Escape closes the menu and returns focus to the row that opened it
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(rows.nth(1)).toBeFocused();
});

test("opens the torrent menu on screen from a card's More actions button", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.reload();
  await page.getByRole("button", { name: "More actions" }).first().click();

  const menu = page.getByRole("menu", { name: "Torrent actions" });
  await expect(menu).toBeVisible();
  const box = await menu.boundingBox();
  const viewport = page.viewportSize()!;
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width);
  expect(box!.y + box!.height).toBeLessThanOrEqual(viewport.height);
});
