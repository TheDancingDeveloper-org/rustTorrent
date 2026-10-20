import { expect, test } from "@playwright/test";

// UI fix WP-G: settings tabs are an accessible, scrollable tab list.

test.beforeEach(async ({ page }) => {
  await page.goto("/mock.html");
});

test("settings dialog is named and its tabs follow the tab pattern", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await expect(dialog).toBeVisible();

  const tablist = dialog.getByRole("tablist", { name: "Settings" });
  const tabs = tablist.getByRole("tab");
  const first = tabs.first();
  await expect(first).toHaveAttribute("aria-selected", "true");
  await expect(dialog.getByRole("tabpanel")).toHaveCount(1);
  const panelId = await first.getAttribute("aria-controls");
  await expect(dialog.locator(`[id="${panelId}"]`)).toHaveAttribute(
    "role",
    "tabpanel",
  );

  // Only the selected tab is in the tab order; arrows move and select
  await first.focus();
  await page.keyboard.press("ArrowRight");
  const second = tabs.nth(1);
  await expect(second).toBeFocused();
  await expect(second).toHaveAttribute("aria-selected", "true");
  await expect(first).toHaveAttribute("tabindex", "-1");
  await expect(dialog.getByRole("tabpanel")).toHaveAttribute(
    "aria-labelledby",
    (await second.getAttribute("id"))!,
  );

  await page.keyboard.press("ArrowLeft");
  await expect(first).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("End");
  await expect(tabs.last()).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("ArrowRight");
  await expect(first).toHaveAttribute("aria-selected", "true");
});

test("settings tabs scroll at 390px and keep the active tab visible", async ({
  page,
}) => {
  // The Settings button is in the collapsed toolbar at 390px, so open the
  // dialog wide and then narrow the viewport
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.setViewportSize({ width: 390, height: 844 });
  const tablist = page
    .getByRole("dialog", { name: "Settings" })
    .getByRole("tablist");

  const overflows = await tablist.evaluate(
    (el) => el.scrollWidth > el.clientWidth,
  );
  expect(overflows).toBe(true);

  await tablist.getByRole("tab").first().focus();
  await page.keyboard.press("End");
  const last = tablist.getByRole("tab").last();
  await expect(last).toHaveAttribute("aria-selected", "true");
  // Scrolled horizontally into the list's visible area
  await expect
    .poll(async () => {
      const list = (await tablist.boundingBox())!;
      const tab = (await last.boundingBox())!;
      return (
        tab.x >= list.x - 1 && tab.x + tab.width <= list.x + list.width + 1
      );
    })
    .toBe(true);
});
