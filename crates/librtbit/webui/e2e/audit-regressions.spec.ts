import { expect, Page, test } from "@playwright/test";

// Regression guards for the 2026-10-17 UI audit findings
// (docs/ui-audit-2026-10-17.md). Each test names the finding IDs it holds
// closed, so a failure points straight at the audit entry it reopens.
// Findings already guarded by another spec are listed in
// docs/ui-audit-verification-2026-10-20.md, not repeated here.

const DESKTOP = { width: 1440, height: 900 };
const MOBILE = { width: 390, height: 844 };

const openMock = async (page: Page, viewport = DESKTOP) => {
  await page.setViewportSize(viewport);
  await page.goto("/mock.html");
  await expect(page).toHaveTitle(/rustTorrent/i);
  if (viewport.width >= 1024) {
    await expect(page.locator("tbody tr").first()).toBeVisible();
  } else {
    await expect(
      page.getByRole("button", { name: "More actions" }).first(),
    ).toBeVisible();
  }
};

// Visible interactive elements with no accessible name, described for the
// failure message. Uses the same rule as the audit's DOM probe: text content,
// aria-label, aria-labelledby or an associated <label> counts; title does not.
const unnamedControls = (page: Page) =>
  page.evaluate(() => {
    const selector =
      'button, a[href], input:not([type="hidden"]), select, textarea, [role="button"], [role="menuitem"], [role="tab"], [role="separator"][tabindex]';
    const isVisible = (el: Element) => {
      const r = el.getBoundingClientRect();
      const s = getComputedStyle(el);
      return (
        r.width > 0 &&
        r.height > 0 &&
        s.visibility !== "hidden" &&
        s.display !== "none"
      );
    };
    const hasName = (el: HTMLElement) => {
      if (el.getAttribute("aria-label")?.trim()) return true;
      const labelledBy = el.getAttribute("aria-labelledby");
      if (
        labelledBy &&
        labelledBy
          .split(/\s+/)
          .some((id) => document.getElementById(id)?.textContent?.trim())
      )
        return true;
      if (
        el instanceof HTMLInputElement ||
        el instanceof HTMLSelectElement ||
        el instanceof HTMLTextAreaElement
      ) {
        if (el.labels && el.labels.length > 0) return true;
        return false;
      }
      return Boolean(el.textContent?.trim());
    };
    return Array.from(document.querySelectorAll<HTMLElement>(selector))
      .filter(isVisible)
      .filter((el) => !hasName(el))
      .map((el) => el.outerHTML.slice(0, 160));
  });

for (const [name, viewport] of [
  ["desktop", DESKTOP],
  ["mobile", MOBILE],
] as const) {
  test(`P0-3 P1-2: every visible control on the ${name} torrent view has a name`, async ({
    page,
  }) => {
    await openMock(page, viewport);
    expect(await unnamedControls(page)).toEqual([]);
  });

  test(`P2-16 P2-17: the ${name} view has one h1 and the page landmarks`, async ({
    page,
  }) => {
    await openMock(page, viewport);
    await expect(page.locator("h1")).toHaveCount(1);
    await expect(page.getByRole("banner")).toHaveCount(1);
    await expect(page.getByRole("main")).toHaveCount(1);
    if (viewport.width >= 1024) {
      await expect(
        page.getByRole("complementary", { name: "Filters" }),
      ).toBeVisible();
    }
    // Headings never skip a level below the page h1
    const levels = await page
      .locator("h1, h2, h3, h4, h5, h6")
      .evaluateAll((els) =>
        els
          .filter((el) => el.getClientRects().length > 0 || el.matches("h1"))
          .map((el) => Number(el.tagName[1])),
      );
    for (let i = 1; i < levels.length; i++) {
      expect(levels[i] - levels[i - 1]).toBeLessThanOrEqual(1);
    }
  });

  test(`no page-level horizontal scroll on the ${name} view`, async ({
    page,
  }) => {
    await openMock(page, viewport);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
}

test("P0-4 P2-7: card actions are real buttons with distinct names", async ({
  page,
}) => {
  await openMock(page, MOBILE);
  const card = page
    .getByRole("button", { name: "More actions" })
    .first()
    .locator("xpath=ancestor::*[.//button[@aria-label][2]][1]");
  const names = await card
    .locator("button[aria-label]")
    .evaluateAll((els) => els.map((el) => el.getAttribute("aria-label")));
  expect(names.length).toBeGreaterThanOrEqual(3);
  expect(new Set(names).size).toBe(names.length);
  // No card action is a link styled as a button
  await expect(card.locator('a[href="#"]')).toHaveCount(0);
});

test("P1-4: sort headers are buttons and announce their sort state", async ({
  page,
}) => {
  await openMock(page);
  const sortable = page.locator('th[role="columnheader"][aria-sort]');
  expect(await sortable.count()).toBeGreaterThan(2);
  const header = sortable.filter({ has: page.getByRole("button") }).nth(1);
  const button = header.getByRole("button");

  await button.focus();
  await page.keyboard.press("Enter");
  await expect(header).toHaveAttribute("aria-sort", /ascending|descending/);
  const first = await header.getAttribute("aria-sort");
  await page.keyboard.press("Enter");
  await expect(header).not.toHaveAttribute("aria-sort", first!);
  // Only one column is sorted at a time
  await expect(
    page.locator('th[aria-sort="ascending"], th[aria-sort="descending"]'),
  ).toHaveCount(1);
});

test("P1-5: columns resize from the keyboard and the handle is at least 12px wide", async ({
  page,
}) => {
  await openMock(page);
  const handle = page.getByRole("separator", { name: /^Resize .* column$/ });
  const first = handle.first();
  const box = await first.boundingBox();
  expect(box!.width).toBeGreaterThanOrEqual(12);

  const th = first.locator("xpath=ancestor::th[1]");
  const before = (await th.boundingBox())!.width;
  await first.focus();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await expect
    .poll(async () => (await th.boundingBox())!.width)
    .toBeGreaterThan(before);
});

test("P1-8: the torrent menu stays on screen in a short window", async ({
  page,
}) => {
  await openMock(page, { width: 1024, height: 420 });
  const row = page.locator('[role="row"][data-row-index]').first();
  await row.click({ button: "right", position: { x: 200, y: 10 } });

  const menu = page.getByRole("menu", { name: "Torrent actions" });
  await expect(menu).toBeVisible();
  const box = (await menu.boundingBox())!;
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.y + box.height).toBeLessThanOrEqual(420);
  // The first item is reachable, not clipped above the viewport
  await expect(menu.locator('[role^="menuitem"]').first()).toBeInViewport();
});

test("P2-18: dialogs are named by their heading", async ({ page }) => {
  await openMock(page);
  await page.getByRole("button", { name: /add torrent from magnet/i }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  const labelledBy = await dialog.getAttribute("aria-labelledby");
  expect(labelledBy).toBeTruthy();
  await expect(page.locator(`[id="${labelledBy}"]`)).toHaveText(/\S/);
});

test("P1-10: primary touch targets are at least 24px on a coarse pointer", async ({
  browser,
}) => {
  const context = await browser.newContext({
    viewport: MOBILE,
    hasTouch: true,
    isMobile: true,
  });
  const page = await context.newPage();
  await page.goto("/mock.html");
  await expect(
    page.getByRole("button", { name: "More actions" }).first(),
  ).toBeVisible();

  const small = await page.evaluate(() =>
    Array.from(
      document.querySelectorAll<HTMLElement>("header button, main button"),
    )
      .filter((el) => el.getClientRects().length > 0)
      .map((el) => ({ el, r: el.getBoundingClientRect() }))
      .filter(({ r }) => r.width < 24 || r.height < 24)
      .map(
        ({ el, r }) =>
          `${el.getAttribute("aria-label") ?? el.textContent?.trim()} ${Math.round(r.width)}x${Math.round(r.height)}`,
      ),
  );
  expect(small).toEqual([]);
  await context.close();
});
