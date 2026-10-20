import { expect, Page, Route, test } from "@playwright/test";

// The real app shell (index.html on the dev server) against a routed backend:
// the ?magnet= deep link and the session-expired path, which the mock UI
// doesn't exercise.
test.use({ baseURL: "http://127.0.0.1:3031" });

const API = "http://127.0.0.1:3030";
const MAGNET = "magnet:?xt=urn:btih:3b245504cf5f11bbdbe1201cea6a6bf45aee1bc0";
const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Headers": "*",
  "Access-Control-Allow-Methods": "*",
};

type Backend = { auth: boolean; torrentsStatus: number };

const routeBackend = async (page: Page, backend: Backend) => {
  await page.route(`${API}/**`, async (route: Route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (request.method() === "OPTIONS") {
      return route.fulfill({ status: 204, headers: CORS });
    }
    if (url.pathname === "/auth/status") {
      return route.fulfill({
        status: 200,
        headers: CORS,
        json: { auth_enabled: backend.auth, setup_required: false },
      });
    }
    if (url.pathname === "/auth/refresh") {
      return route.fulfill({ status: 401, headers: CORS, body: "expired" });
    }
    if (url.pathname === "/torrents" && request.method() === "GET") {
      return backend.torrentsStatus === 200
        ? route.fulfill({ status: 200, headers: CORS, json: { torrents: [] } })
        : route.fulfill({
            status: backend.torrentsStatus,
            headers: CORS,
            body: "unauthorized",
          });
    }
    if (url.pathname === "/torrents" && request.method() === "POST") {
      return route.fulfill({ status: 500, headers: CORS, body: "test stub" });
    }
    return route.abort("connectionrefused");
  });
};

const isListOnlyUpload = (request: { method(): string; url(): string }) =>
  request.method() === "POST" &&
  new URL(request.url()).pathname === "/torrents" &&
  request.url().includes("list_only=true");

test("a ?magnet= deep link starts the add flow and is removed from the URL", async ({
  page,
}) => {
  await routeBackend(page, { auth: false, torrentsStatus: 200 });
  const upload = page.waitForRequest(isListOnlyUpload);
  await page.goto(`/?magnet=${encodeURIComponent(MAGNET)}`);

  const request = await upload;
  expect(request.postData()).toContain(MAGNET);
  await expect.poll(() => page.url()).not.toContain("magnet=");
});

test("an invalid ?magnet= deep link says so and uploads nothing", async ({
  page,
}) => {
  await routeBackend(page, { auth: false, torrentsStatus: 200 });
  let uploads = 0;
  page.on("request", (r) => {
    if (isListOnlyUpload(r)) uploads++;
  });
  await page.goto("/?magnet=not-a-magnet");

  await expect(page.getByText("That magnet link is not valid")).toBeVisible();
  await expect.poll(() => page.url()).not.toContain("magnet=");
  expect(uploads).toBe(0);
});

test("a rejected session returns to the login page with an expiry notice", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("rtbit_access_token", "stale-access");
    localStorage.setItem("rtbit_refresh_token", "stale-refresh");
    localStorage.setItem("rtbit_token_expiry", String(Date.now() + 3_600_000));
  });
  await routeBackend(page, { auth: true, torrentsStatus: 401 });
  await page.goto("/");

  await expect(
    page
      .getByRole("status")
      .filter({ hasText: "Your session expired. Sign in again to continue." }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: /sign in/i })).toBeVisible();
  // The rejected tokens are gone, so a reload doesn't retry them
  expect(
    await page.evaluate(() => localStorage.getItem("rtbit_access_token")),
  ).toBeNull();
});
