# rustTorrent web UI audit verification — 2026-10-20

Phase 3 of 3 for [`ui-audit-2026-10-17.md`](ui-audit-2026-10-17.md): re-run the
capture and check every finding ID against the fixes from
[`ui-fix-handoff-2026-10-17.md`](ui-fix-handoff-2026-10-17.md) (WP-A to WP-H).
Checked against `origin/main` at `39091e6` plus this branch. WP-H (P2-21 to
P2-23) was checked on `wi-1924-desktop-splash` (PR #159), which has not merged
yet.

## Method

- **Captures:** `design/audits/2026-10-20/` holds the same 32 screenshots as
  the 2026-10-17 set: the same names, 1440×900 and 390×844 (touch), light and
  dark. They come from `e2e/audit-capture.spec.ts`, so the next audit can repeat
  them with one command (see `docs/TESTING.md`). The no-daemon view now uses the
  real entry point with every engine call refused, so it shows the unreachable
  state on purpose rather than whatever the dev server returns.
- **DOM guards:** `e2e/audit-regressions.spec.ts` holds the DOM-checkable
  findings closed. Each test names the IDs it covers. Findings that an existing
  spec already guards are listed in the table below rather than repeated.
- **Code read:** the other findings were checked by reading the cited code.

## Fixed in this pass

The regression spec found these, and this branch fixes them:

- **P0-3:** three icon-only buttons had no accessible name, only a `title`:
  "New category" (`compact/CategoryFilter.tsx`), the footer alt-speed toggle
  (`Footer.tsx`, which now also sets `aria-pressed`) and the error alert's close
  button (`ErrorComponent.tsx`).
- **P1-4:** pressing a sort header a second time didn't flip the direction in
  StrictMode builds (`npm run dev`, the mock UI and the e2e suite).
  `TorrentTable` and `PeersTab` set the direction from inside the column's state
  updater, and StrictMode runs updaters twice, so the toggle cancelled itself.
  Both now use the pure `helper/sortToggle.ts` (with node tests).
- **P1-5:** the resize handle was `w-3`. That class is rem-based and the root
  font is 14px, so the hit area was 10.5px, under the recommended 12px. It is
  now `w-[12px]`.
- **P1-1:** five inputs outside the WP-F/G file sets still had
  `focus:outline-none` with no ring: Indexarr search, log filter, upload modal,
  Overview tab and the new-category input. They now use `focus-ring`.
  `helper/focusIndicator.test.mjs` fails on any class string that drops the
  outline without putting an indicator back.
- **P1-10:** the hamburger is now a `touch-target`, and the search clear buttons
  have a focus ring.

## Finding status

Fixed = closed and checked. Partial = some of the listed locations remain.
Open = not started. "Guard" names the test that holds the finding closed.

| ID | Status | Evidence | Guard |
|---|---|---|---|
| P0-1 | Fixed | Mobile drawer has Torrents/Index/RSS buttons (`rtbit-web.tsx`) | `mobile-navigation.spec.ts` |
| P0-2 | Fixed | `role="grid"`, roving `tabIndex` rows, arrows/Space/Enter/Shift+F10 | `mock-ui.spec.ts` keyboard tests |
| P0-3 | Fixed | All icon controls labelled; last three fixed in this pass | `audit-regressions` P0-3 (desktop + mobile) |
| P0-4 | Fixed | `IconButton` renders `<button type="button" disabled aria-label>` | `audit-regressions` P0-4 |
| P1-1 | Fixed | `focus-ring` on every input; last five fixed in this pass | `focusIndicator.test.mjs` |
| P1-2 | Fixed | Search, status and sort controls have `aria-label` | `audit-regressions` P1-2 |
| P1-3 | Fixed | Tertiary is slate-500 / slate-400 | `tertiaryContrast.test.mjs` |
| P1-4 | Fixed | `<button>` in `<th aria-sort>`; toggle fixed in this pass | `audit-regressions` P1-4, `sortToggle.test.mjs` |
| P1-5 | Fixed | `role="separator"`, pointer capture, arrow keys, 12px | `audit-regressions` P1-5 |
| P1-6 | Partial | Cards: "More actions" button and long-press. Table rows still open the menu only by right-click or Shift+F10; there is no touch path at `lg`+ | `mock-ui.spec.ts` card menu |
| P1-7 | Fixed | `role="menu"`, `menuitem`s, focus in and restored, arrow keys | `mock-ui.spec.ts` keyboard menu |
| P1-8 | Fixed | Clamp measured after mount, `max-h` and scroll | `audit-regressions` P1-8, `mock-ui.spec.ts` |
| P1-9 | Fixed | Short "Magnet"/"Upload" labels below `sm`; "More options" overflow | capture `01-torrents-mobile-*` |
| P1-10 | Partial | Toolbar icons, `IconButton`, rows and hamburger meet 44px on coarse pointers. The search clear buttons are about 26px: they pass WCAG 2.5.8 (24px) but not the 44px guideline, because they sit inside the input | `audit-regressions` P1-10 (24px floor) |
| P1-11 | Fixed | Tab list scrolls with snap and keeps the active tab in view | `settings-tabs.spec.ts` |
| P1-12 | Fixed | `EngineUnreachable` panel with API URL, retry status and Retry | `add-flow-wiring.spec.ts` (new test) |
| P1-13 | Partial | `customSetInterval` backs off while hidden. These still poll with raw `setInterval`: `Footer.tsx` (10 s), `RSSPage.tsx` (30 s), `IndexarrBrowse.tsx` (60 s) | `customSetInterval.test.mjs` |
| P2-1 | Fixed | SpeedTab derives its history from the store | — |
| P2-2 | Fixed | `refreshTorrents` is a `useCallback` with real deps | — |
| P2-3 | Partial | RSS and Indexarr are lazy; LogStream, the config tabs and the SpeedTab chart are not | — |
| P2-4 | Fixed | `handleCardContextMenu` is a memoised callback | — |
| P2-5 | Fixed | Literal colour lookup in `IconButton` | — |
| P2-6 | Partial | The listed blue sources are fixed. The desktop table progress fill still uses `bg-accent-download` (blue) while card bars are orange (`TorrentTableRow.tsx`) | — |
| P2-7 | Fixed | Resting action colours are success/secondary/secondary/error | `audit-regressions` P0-4 (distinct names) |
| P2-8 | Fixed | Settings stats use `text-text font-medium tabular-nums` | `settingsTheming.test.mjs` |
| P2-9 | Fixed | Feature pages use the status tokens. `LogLine.tsx`, which was outside the listed files, still uses raw palette colours | `featurePageTokens.test.mjs` |
| P2-10 | Fixed | Speed chart reads the CSS accent variables | — |
| P2-11 | Fixed | Indexarr badges have AA light and dark steps | `featurePageTokens.test.mjs` |
| P2-12 | Fixed | Full-track label, no `false` class, `role="progressbar"` | — |
| P2-13 | Fixed | Drawer `w-72`, Close button, Escape, no Collapse | `mobile-navigation.spec.ts` |
| P2-14 | Fixed | Inputs shrink (`min-w-0`, `max-w-*`) | `audit-regressions` horizontal scroll |
| P2-15 | Fixed | RSS lists stack below `sm` | `rss-narrow-layout.spec.ts` |
| P2-16 | Fixed | sr-only `h1`; `h2` section headings; no skipped levels | `audit-regressions` P2-16 |
| P2-17 | Fixed | `<header>`, `<aside aria-label="Filters">`, `<main>`, `<footer>`; the page tabs have no `<nav>` wrapper | `audit-regressions` P2-17 |
| P2-18 | Fixed | `Modal` links the `h2` with `aria-labelledby` | `audit-regressions` P2-18, `settings-tabs.spec.ts` |
| P2-19 | Fixed | Spinner `role="status"`, sr-only text, reduced motion | — |
| P2-20 | Fixed | Filter rows are `aria-pressed` buttons; counts pass contrast | — |
| P2-21 | Fixed in PR #159 | Splash light/dark tokens, orange accent, "rustTorrent" | `desktop-splash.spec.ts` (PR #159) |
| P2-22 | Fixed in PR #159 | `role="status" aria-live`, reduced-motion pulse | `desktop-splash.spec.ts` (PR #159) |
| P2-23 | Fixed in PR #159 | Fails after 20 s with a hint and Retry | `desktop-splash.spec.ts` (PR #159) |
| P3-1 | Fixed | `useListOnlyUpload` with a cancelled guard | — |
| P3-2 | Partial | One input constant per file in RSS; there is still no shared input primitive, and about 20 copies remain | — |
| P3-3 | Open | Four button systems remain (`toolbarIcon*`, `btnSmCls`, `Button`, `IconButton`) | — |
| P3-4 | Partial | The search field radius is unified; global radius drift remains | — |
| P3-5 | Partial | Placeholders and aria-labels are consistent. The tooltips still say "Configure", "Logout" and "Toggle dark mode", and the "Add torrent" modal title doesn't match its button | — |
| P3-6 | Fixed | `TableHeader.tsx` and `AdvancedTab.tsx` deleted | — |
| P3-7 | Fixed | Clear buttons have `type="button"` | — |
| P3-8 | Open | The empty detail pane still takes full height (`compact/DetailPane.tsx`, `compact/CompactLayout.tsx` were outside WP-E's files) | — |

**Totals:** 35 fixed (3 of them pending PR #159), 10 partial, 2 open. All P0
findings are closed.

## Left open

These need a follow-up item. None of them is a P0:

1. **P1-6:** add a touch path to the table-row menu: a "⋯" button on row focus
   or hover, or long-press.
2. **P1-13:** move the footer, RSS and Indexarr polls onto `customSetInterval`.
3. **P2-6:** decide whether the table progress fill should be the download
   accent or the brand colour, and make cards and rows agree.
4. **P2-3:** lazy-load LogStream, the config tabs and the speed chart.
5. **P3-2 to P3-5, P3-8:** shared input and button primitives, radius
   scale, tooltip copy, and a collapsed empty detail pane.
6. **P1-10:** the search clear buttons' touch size, if 44px is wanted rather
   than the AA minimum.
