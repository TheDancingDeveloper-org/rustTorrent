import { expect, test, type Page } from "@playwright/test";

// UI fix WP-H: the desktop splash (desktop/src/index.html), opened straight
// from disk with the engine on 127.0.0.1:3030 stubbed.

const SPLASH = new URL("../../../../desktop/src/index.html", import.meta.url)
  .href;
const ENGINE = "http://127.0.0.1:3030";
const CORS = { "Access-Control-Allow-Origin": "*" };

// Records every non-file request; the engine is down unless `up` is set.
async function stubEngine(page: Page, state: { up: boolean }) {
  const requests: string[] = [];
  await page.route(
    (url) => url.protocol !== "file:",
    async (route) => {
      const url = route.request().url();
      requests.push(url);
      if (!url.startsWith(`${ENGINE}/`) || !state.up) {
        return route.abort("connectionrefused");
      }
      if (url === `${ENGINE}/stats`) {
        return route.fulfill({ status: 200, headers: CORS, json: {} });
      }
      return route.fulfill({
        status: 200,
        contentType: "text/html",
        body: "<title>engine web ui</title>",
      });
    },
  );
  return requests;
}

test("shows a branded, announced starting state", async ({ page }) => {
  await stubEngine(page, { up: false });
  await page.goto(SPLASH);

  await expect(page).toHaveTitle("rustTorrent");
  await expect(page.getByRole("status")).toHaveText(
    "Starting the rustTorrent engine…",
  );
  await expect(page.getByRole("status")).toHaveAttribute("aria-live", "polite");
  await expect(page.getByRole("button", { name: "Retry" })).toBeHidden();
});

test("redirects to the web UI once the engine answers", async ({ page }) => {
  const state = { up: false };
  const requests = await stubEngine(page, state);
  await page.goto(SPLASH);
  await expect(page.getByRole("status")).toContainText("Starting");

  state.up = true;
  await page.waitForURL(`${ENGINE}/web/`);
  await expect(page).toHaveTitle("engine web ui");
  for (const url of requests) {
    expect(url.startsWith(`${ENGINE}/`), url).toBe(true);
  }
});

test("shows the failure state after 20 s and retries", async ({ page }) => {
  await page.clock.install();
  const state = { up: false };
  await stubEngine(page, state);
  await page.goto(SPLASH);
  await expect(page.getByRole("status")).toContainText("Starting");

  await page.clock.fastForward(19_000);
  await expect(page.getByRole("button", { name: "Retry" })).toBeHidden();

  await page.clock.fastForward(1_500);
  const retry = page.getByRole("button", { name: "Retry" });
  await expect(page.getByRole("status")).toHaveText(
    "The rustTorrent engine didn't start",
  );
  await expect(retry).toBeVisible();
  await expect(retry).toBeFocused();
  await expect(page.getByText("from a terminal to see the engine log")).toBeVisible();

  // Retry restarts polling; with the engine up it then redirects
  state.up = true;
  await retry.click();
  await expect(retry).toBeHidden();
  await page.clock.runFor(1_000);
  await page.waitForURL(`${ENGINE}/web/`);
});

for (const [scheme, surface] of [
  ["light", "rgb(248, 250, 252)"],
  ["dark", "rgb(15, 23, 42)"],
] as const) {
  test(`follows the ${scheme} colour scheme`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await stubEngine(page, { up: false });
    await page.goto(SPLASH);
    await expect(page.locator("body")).toHaveCSS("background-color", surface);
  });
}

test("replaces the spin with a pulse for reduced motion", async ({ page }) => {
  await stubEngine(page, { up: false });
  await page.goto(SPLASH);
  await expect(page.locator("#spinner")).toHaveCSS("animation-name", "spin");

  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(page.locator("#spinner")).toHaveCSS("animation-name", "pulse");
});
