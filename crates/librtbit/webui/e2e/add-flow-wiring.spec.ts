import { expect, Page, Route, test } from "@playwright/test";

// Runs the real app shell (index.html on the dev server) against a routed
// backend, so the paste, drop and connection-lost wiring is exercised without a
// running rustTorrent engine.
test.use({ baseURL: "http://127.0.0.1:3031" });

const API = "http://127.0.0.1:3030";
const MAGNET = "magnet:?xt=urn:btih:3b245504cf5f11bbdbe1201cea6a6bf45aee1bc0";

type Mode = "ok" | "down";

const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Headers": "*",
  "Access-Control-Allow-Methods": "*",
};

// Torrent list answers "ok" with an empty library, or fails like an unreachable
// engine. Upload calls are recorded by the test and answered with a 500 so the
// file-selection modal stays in its simple error state.
const routeBackend = async (page: Page, mode: () => Mode) => {
  await page.route(`${API}/**`, async (route: Route) => {
    const request = route.request();
    const url = new URL(request.url());

    if (request.method() === "OPTIONS") {
      return route.fulfill({ status: 204, headers: CORS });
    }
    if (url.pathname === "/torrents" && request.method() === "GET") {
      if (mode() === "down") return route.abort("connectionrefused");
      return route.fulfill({
        status: 200,
        headers: CORS,
        contentType: "application/json",
        body: JSON.stringify({ torrents: [] }),
      });
    }
    if (url.pathname === "/torrents" && request.method() === "POST") {
      return route.fulfill({ status: 500, headers: CORS, body: "test stub" });
    }
    // Everything else (auth status, stats, version, ...) fails quietly; the
    // auth gate treats a failed status check as "no auth".
    return route.abort("connectionrefused");
  });
};

const openApp = async (page: Page, mode: () => Mode) => {
  await routeBackend(page, mode);
  await page.goto("/");
  await expect(page.getByText("No torrents yet")).toBeVisible();
};

const dispatchPaste = (page: Page, text: string) =>
  page.evaluate((value) => {
    const dataTransfer = new DataTransfer();
    dataTransfer.setData("text/plain", value);
    document.body.dispatchEvent(
      new ClipboardEvent("paste", {
        clipboardData: dataTransfer,
        bubbles: true,
        cancelable: true,
      }),
    );
  }, text);

const dispatchDrop = (page: Page, setData: [string, string][]) =>
  page.evaluate((entries) => {
    const dataTransfer = new DataTransfer();
    for (const [type, value] of entries) dataTransfer.setData(type, value);
    document.querySelector("main")!.dispatchEvent(
      new DragEvent("drop", {
        dataTransfer,
        bubbles: true,
        cancelable: true,
      }),
    );
  }, setData);

const isListOnlyUpload = (request: { method(): string; url(): string }) =>
  request.method() === "POST" &&
  new URL(request.url()).pathname === "/torrents" &&
  request.url().includes("list_only=true");

test("pasting a magnet link starts the add flow", async ({ page }) => {
  let mode: Mode = "ok";
  await openApp(page, () => mode);

  const upload = page.waitForRequest(isListOnlyUpload);
  await dispatchPaste(page, MAGNET);
  expect((await upload).postData()).toBe(MAGNET);
});

test("dropping a magnet link starts the add flow", async ({ page }) => {
  let mode: Mode = "ok";
  await openApp(page, () => mode);

  const upload = page.waitForRequest(isListOnlyUpload);
  await dispatchDrop(page, [["text/uri-list", MAGNET]]);
  expect((await upload).postData()).toBe(MAGNET);
});

test("dropping a non-torrent file says why it was refused", async ({
  page,
}) => {
  let mode: Mode = "ok";
  await openApp(page, () => mode);

  await page.evaluate(() => {
    const dataTransfer = new DataTransfer();
    dataTransfer.items.add(
      new File(["x"], "notes.txt", { type: "text/plain" }),
    );
    document.querySelector("main")!.dispatchEvent(
      new DragEvent("drop", {
        dataTransfer,
        bubbles: true,
        cancelable: true,
      }),
    );
  });
  await expect(
    page.getByText("Only .torrent files can be added"),
  ).toBeVisible();
});

test("the connection-lost banner shows, keeps the list, and hides on recovery", async ({
  page,
}) => {
  let mode: Mode = "ok";
  await openApp(page, () => mode);

  const banner = page
    .getByRole("status")
    .filter({ hasText: "Connection to rustTorrent lost" });
  await expect(banner).toHaveCount(0);

  // The shell polls every few seconds; the failure is picked up on the next poll
  mode = "down";
  await expect(banner).toBeVisible({ timeout: 15_000 });
  // The last known list stays on screen behind the banner
  await expect(page.getByText("No torrents yet")).toBeVisible();

  mode = "ok";
  await banner.getByRole("button", { name: "Retry" }).click();
  await expect(banner).toHaveCount(0);
  await expect(
    page.getByRole("status").getByText("Connection to rustTorrent restored"),
  ).toBeVisible();
});

// Audit P1-12: with no engine from the start, the shell shows a dedicated
// unreachable state with a Retry, not a raw HTTP error over an empty list.
test("an unreachable engine shows the full-panel state, and Retry recovers", async ({
  page,
}) => {
  let mode: Mode = "down";
  await routeBackend(page, () => mode);
  await page.goto("/");

  const panel = page
    .getByRole("heading", { name: "Can't reach the rustTorrent engine" })
    .locator("xpath=..");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText(API);
  await expect(
    panel.getByRole("status").filter({ hasText: "Retrying automatically" }),
  ).toBeVisible();
  await expect(page.getByText("No torrents yet")).toHaveCount(0);

  mode = "ok";
  await panel.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByText("No torrents yet")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Can't reach the rustTorrent engine" }),
  ).toHaveCount(0);
});
