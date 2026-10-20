import { expect, Page, test } from "@playwright/test";

// Keyboard paths from the polish pass (WI-1732, UI fix WP-B/C/D) that had no
// test: global shortcuts, menu navigation, and the narrow-screen drawer and
// toolbar overflow.

const rows = (page: Page) => page.locator('[role="row"][data-row-index]');

const openMock = async (page: Page, width = 1440, height = 900) => {
  await page.setViewportSize({ width, height });
  await page.goto("/mock.html");
  await expect(page).toHaveTitle(/rustTorrent/i);
};

test.describe("global shortcuts", () => {
  test.beforeEach(async ({ page }) => {
    await openMock(page);
    await expect(rows(page).first()).toBeVisible();
    // Start from the page, not a control that handles keys itself
    await page.locator("body").click({ position: { x: 700, y: 5 } });
  });

  test("Ctrl+A selects every visible row and Escape clears it", async ({
    page,
  }) => {
    await page.locator("[data-search-input]").fill("NebulaOS");
    await page.locator("body").click({ position: { x: 700, y: 5 } });
    await expect(rows(page).first()).toContainText("NebulaOS");

    await page.keyboard.press("ControlOrMeta+a");
    const count = await rows(page).count();
    expect(count).toBeGreaterThan(0);
    await expect(
      rows(page).locator('xpath=self::*[@aria-selected="true"]'),
    ).toHaveCount(count);

    await page.keyboard.press("Escape");
    await expect(
      rows(page).locator('xpath=self::*[@aria-selected="true"]'),
    ).toHaveCount(0);
  });

  test("Ctrl+F focuses the search field", async ({ page }) => {
    await page.keyboard.press("ControlOrMeta+f");
    await expect(page.locator("[data-search-input]")).toBeFocused();
  });

  test("M opens the add-magnet dialog with its field focused", async ({
    page,
  }) => {
    await page.keyboard.press("m");
    const dialog = page.getByRole("dialog", { name: "Add torrent" });
    await expect(dialog).toBeVisible();
    await expect(dialog.locator('input[placeholder^="magnet:"]')).toBeFocused();
  });

  test("M does not reopen the magnet dialog after leaving and returning", async ({
    page,
  }) => {
    // The toolbar outlives the torrents page, so its press counter is still
    // above zero when the page remounts. Only a new press may open the dialog.
    await page.keyboard.press("m");
    const dialog = page.getByRole("dialog", { name: "Add torrent" });
    await expect(dialog).toBeVisible();
    await page.getByRole("button", { name: "Close modal" }).click();
    await expect(dialog).toHaveCount(0);

    await page.getByRole("button", { name: "RSS" }).click();
    await page.getByRole("button", { name: "Torrents" }).click();
    await expect(rows(page).first()).toBeVisible();
    await expect(dialog).toHaveCount(0);

    // The shortcut itself still works after the round trip
    await page.locator("body").click({ position: { x: 700, y: 5 } });
    await page.keyboard.press("m");
    await expect(dialog).toBeVisible();
  });

  test("Ctrl+, opens settings and Ctrl+L opens the logs", async ({ page }) => {
    await page.keyboard.press("ControlOrMeta+,");
    await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);

    await page.keyboard.press("ControlOrMeta+l");
    await expect(page.getByRole("dialog")).toBeVisible();
  });

  test("B collapses and expands the sidebar", async ({ page }) => {
    const expand = page.getByRole("button", { name: "Expand sidebar" });
    const collapse = page.getByRole("button", { name: "Collapse sidebar" });
    await expect(collapse).toBeVisible();
    await page.keyboard.press("b");
    await expect(expand).toBeVisible();
    await expect(collapse).toHaveCount(0);
    await page.keyboard.press("b");
    await expect(collapse).toBeVisible();
  });

  test("Enter on a single selection opens its details", async ({ page }) => {
    await rows(page).nth(2).click();
    await page.locator("body").click({ position: { x: 700, y: 5 } });
    // Re-select without leaving focus on the row, so the global handler acts
    await page.keyboard.press("Escape");
    await rows(page).nth(2).click();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("tab", { name: "Overview" })).toBeVisible();
  });

  test("shortcuts are ignored while typing in a field", async ({ page }) => {
    const search = page.locator("[data-search-input]");
    await search.click();
    await page.keyboard.type("mb");
    await expect(search).toBeFocused();
    await expect(search).toHaveValue("mb");
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });
});

test("the torrent menu moves with arrows, Home and End, and Tab closes it", async ({
  page,
}) => {
  await openMock(page);
  await rows(page).nth(0).focus();
  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu", { name: "Torrent actions" });
  await expect(menu).toBeVisible();

  const items = menu.locator('[role^="menuitem"]:not(:disabled)');
  const count = await items.count();
  expect(count).toBeGreaterThan(2);
  const focusedIndex = () =>
    menu.evaluate((el) => {
      const all = Array.from(
        el.querySelectorAll('[role^="menuitem"]:not(:disabled)'),
      );
      return all.indexOf(document.activeElement as Element);
    });

  await expect.poll(focusedIndex).toBe(0);
  await page.keyboard.press("ArrowDown");
  await expect.poll(focusedIndex).toBe(1);
  await page.keyboard.press("End");
  await expect.poll(focusedIndex).toBe(count - 1);
  await page.keyboard.press("ArrowDown");
  await expect.poll(focusedIndex).toBe(0);
  await page.keyboard.press("ArrowUp");
  await expect.poll(focusedIndex).toBe(count - 1);
  await page.keyboard.press("Home");
  await expect.poll(focusedIndex).toBe(0);

  await page.keyboard.press("Tab");
  await expect(menu).toHaveCount(0);
  await expect(rows(page).nth(0)).toBeFocused();
});

test.describe("narrow screens", () => {
  test.beforeEach(async ({ page }) => {
    await openMock(page, 390, 844);
    await expect(
      page.getByRole("button", { name: "More actions" }).first(),
    ).toBeVisible();
  });

  test("the drawer takes focus, closes on Escape and returns focus", async ({
    page,
  }) => {
    const opener = page.getByRole("button", { name: "Open sidebar" });
    await opener.focus();
    await page.keyboard.press("Enter");
    const drawer = page.getByRole("dialog", { name: "Navigation" });
    await expect(drawer).toBeVisible();
    await expect
      .poll(() => drawer.evaluate((el) => el.contains(document.activeElement)))
      .toBe(true);

    await page.keyboard.press("Escape");
    await expect(drawer).toBeHidden();
    await expect(opener).toBeFocused();
  });

  test("More options holds the toolbar actions that don't fit", async ({
    page,
  }) => {
    const more = page.getByRole("button", { name: "More options" });
    await expect(more).toHaveAttribute("aria-expanded", "false");
    await more.click();
    await expect(more).toHaveAttribute("aria-expanded", "true");
    const menu = page.getByRole("menu", { name: "Toolbar options" });
    await expect(
      menu.getByRole("menuitem", { name: "Settings" }),
    ).toBeVisible();
    await expect(
      menu.getByRole("menuitem", { name: "View logs" }),
    ).toBeVisible();

    await menu.getByRole("menuitem", { name: "View logs" }).click();
    await expect(menu).toHaveCount(0);
    await expect(page.getByRole("dialog")).toBeVisible();
  });

  test("the theme toggle flips the page theme", async ({ page }) => {
    const isDark = () =>
      page.evaluate(() => document.body.classList.contains("dark"));
    const before = await isDark();
    await page.getByRole("button", { name: "More options" }).click();
    await page
      .getByRole("menu", { name: "Toolbar options" })
      .getByRole("menuitem", { name: /dark|light|theme/i })
      .click();
    await expect.poll(isDark).toBe(!before);
  });
});
