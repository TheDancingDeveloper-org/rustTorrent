import { expect, test } from "@playwright/test";

test("opens the mobile drawer and navigates to RSS", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/mock.html");
  await expect(page).toHaveTitle(/rustTorrent/i);

  await page.getByTitle("Open sidebar").click();
  const drawer = page.getByRole("dialog", { name: "Navigation" });
  await expect(drawer).toBeVisible();
  await drawer.getByRole("button", { name: "RSS" }).click();

  await expect(drawer).toBeHidden();
  await expect(page.getByRole("button", { name: /^Feeds/ })).toBeVisible();
});
