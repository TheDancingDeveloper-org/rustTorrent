import { expect, Page, test } from "@playwright/test";
import { join } from "node:path";

// Re-takes the UI audit screenshot set (docs/ui-audit-2026-10-17.md, "Screenshot
// index") with the same view names, viewports and colour schemes, so a later
// audit can be compared image by image. Skipped unless AUDIT_CAPTURE_DIR is set:
//
//   AUDIT_CAPTURE_DIR=../../../design/audits/$(date +%F) npx playwright test audit-capture
//
// Uses only the mock UI and, for the no-daemon view, the real entry point with
// every engine call refused. Nothing outside loopback is contacted.

const OUT = process.env.AUDIT_CAPTURE_DIR;
test.skip(!OUT, "set AUDIT_CAPTURE_DIR to capture the audit screenshots");
test.describe.configure({ timeout: 60_000 });

type Viewport = "desktop" | "mobile";
type Scheme = "light" | "dark";

const SIZES = {
  desktop: { width: 1440, height: 900 },
  mobile: { width: 390, height: 844 },
} as const;

const shot = (page: Page, view: string, viewport: Viewport, scheme: Scheme) =>
  page.screenshot({ path: join(OUT!, `${view}-${viewport}-${scheme}.png`) });

const openMock = async (page: Page, viewport: Viewport) => {
  await page.goto("http://127.0.0.1:3032/mock.html");
  await expect(page).toHaveTitle(/rustTorrent/i);
  await expect(
    viewport === "desktop"
      ? page.locator('[role="row"][data-row-index]').first()
      : page.getByRole("button", { name: "More actions" }).first(),
  ).toBeVisible();
};

// Settings and logs sit behind "More options" when the toolbar is narrow.
const openToolbarItem = async (page: Page, name: string) => {
  const direct = page.getByRole("button", { name, exact: true });
  if (await direct.isVisible()) {
    await direct.click();
    return;
  }
  await page.getByRole("button", { name: "More options" }).click();
  await page.getByRole("menuitem", { name }).click();
};

const VIEWS: {
  name: string;
  viewports: Viewport[];
  prepare: (page: Page, viewport: Viewport) => Promise<void>;
}[] = [
  { name: "01-torrents", viewports: ["desktop", "mobile"], prepare: openMock },
  {
    name: "02-detail",
    viewports: ["desktop"],
    prepare: async (page, viewport) => {
      await openMock(page, viewport);
      await page.locator('[role="row"][data-row-index]').nth(1).click();
      await expect(page.getByRole("tab", { name: "Overview" })).toBeVisible();
    },
  },
  {
    name: "03-contextmenu",
    viewports: ["desktop"],
    prepare: async (page, viewport) => {
      await openMock(page, viewport);
      await page
        .locator('[role="row"][data-row-index]')
        .nth(1)
        .click({ button: "right", position: { x: 300, y: 10 } });
      await expect(
        page.getByRole("menu", { name: "Torrent actions" }),
      ).toBeVisible();
    },
  },
  {
    name: "04-rss",
    viewports: ["desktop"],
    prepare: async (page, viewport) => {
      await openMock(page, viewport);
      await page.getByRole("button", { name: "RSS", exact: true }).click();
      await expect(page.getByRole("button", { name: /^Feeds/ })).toBeVisible();
    },
  },
  {
    name: "05-settings",
    viewports: ["desktop", "mobile"],
    prepare: async (page, viewport) => {
      await openMock(page, viewport);
      await openToolbarItem(page, "Settings");
      await expect(page.getByRole("dialog")).toBeVisible();
    },
  },
  {
    name: "06-logs",
    viewports: ["desktop", "mobile"],
    prepare: async (page, viewport) => {
      await openMock(page, viewport);
      await openToolbarItem(page, "View logs");
      await expect(page.getByRole("dialog")).toBeVisible();
    },
  },
  {
    name: "07-add",
    viewports: ["desktop", "mobile"],
    prepare: async (page, viewport) => {
      await openMock(page, viewport);
      await page
        .getByRole("button", { name: /magnet/i })
        .first()
        .click();
      await expect(page.getByRole("dialog")).toBeVisible();
    },
  },
  {
    name: "08-sidebar",
    viewports: ["mobile"],
    prepare: async (page, viewport) => {
      await openMock(page, viewport);
      await page.getByRole("button", { name: "Open sidebar" }).click();
      await expect(
        page.getByRole("dialog", { name: "Navigation" }),
      ).toBeVisible();
    },
  },
  {
    name: "09-focus",
    viewports: ["desktop", "mobile"],
    prepare: async (page, viewport) => {
      await openMock(page, viewport);
      // The audit's probe: focus after six Tab presses from the top
      await page.locator("body").click({ position: { x: 1, y: 1 } });
      for (let i = 0; i < 6; i++) await page.keyboard.press("Tab");
    },
  },
  {
    name: "10-no-daemon",
    viewports: ["desktop", "mobile"],
    prepare: async (page) => {
      await page.route("http://127.0.0.1:3030/**", (route) =>
        route.abort("connectionrefused"),
      );
      await page.goto("http://127.0.0.1:3031/");
      await expect(
        page.getByRole("heading", {
          name: "Can't reach the rustTorrent engine",
        }),
      ).toBeVisible();
    },
  },
];

for (const view of VIEWS) {
  for (const viewport of view.viewports) {
    for (const scheme of ["light", "dark"] as const) {
      test(`${view.name}-${viewport}-${scheme}`, async ({ browser }) => {
        const context = await browser.newContext({
          viewport: SIZES[viewport],
          colorScheme: scheme,
          ...(viewport === "mobile" ? { hasTouch: true, isMobile: true } : {}),
        });
        const page = await context.newPage();
        await view.prepare(page, viewport);
        // Let transitions and the first stats tick settle
        await page.waitForTimeout(500);
        await shot(page, view.name, viewport, scheme);
        await context.close();
      });
    }
  }
}
