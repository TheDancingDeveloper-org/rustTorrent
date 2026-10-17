# rustTorrent web UI fix handoff — 2026-10-17

Phase 2 plan for the findings in [`ui-audit-2026-10-17.md`](ui-audit-2026-10-17.md). Each work package (WP) is sized for one Haiku worker, owns an exclusive set of files, and can be reviewed as its own PR against `dev`.

## Ground rules for every worker

- **Paths:** all are relative to `crates/librtbit/webui/src/` unless they start with `desktop/`.
- **File ownership:** edit **only** the files your package owns. If a fix needs a change in a file you don't own, write the request in your PR description and leave the file alone.
- **Order:** **WP-A lands first.** It adds the shared `focus-ring` utility, the fixed tertiary token and the button primitives that the other packages use. WP-B to WP-G can then run in parallel.
- **Style:** keep the incumbent look. This is refinement, not redesign. Use the semantic tokens in `globals.css` (`text-primary`, `text-secondary`, `text-tertiary`, `bg-surface*`, `border-divider`, `text-success`/`error`/`warning`, `bg-*-bg`). Don't add raw `blue-*`, `gray-*` or hex colours.
- **Naming:** don't use the upstream WebUI project name in code (see the repo `CLAUDE.md`).
- **Checks before opening a PR**, run from `crates/librtbit/webui`:
  ```sh
  npm ci
  npm run lint
  npx tsc --noEmit
  npm test
  npm run test:e2e   # uses the mock UI on 127.0.0.1:3032
  ```
  Then re-take your package's screenshots using the method in the audit (mock UI, 1440 px and 390 px, light and dark), and attach the before/after pairs to the PR.
- **Scope:** use only the mock UI or a local daemon. Never point anything at a deployed instance.

## Package map

| WP | Theme | Owns (exclusive) | Findings | Size | Depends on |
|----|-------|------------------|----------|------|------------|
| A | Tokens and primitives | `globals.css`, `components/buttons/IconButton.tsx`, `components/buttons/TorrentActions.tsx`, `components/buttons/Button.tsx`, `components/buttons/MagnetInput.tsx`, `components/buttons/FileInput.tsx`, `components/forms/FormCheckbox.tsx`, `components/Spinner.tsx`, `components/ProgressBar.tsx` | P0-4, P0-3 (TorrentActions), P1-3, P1-9 (labels), P1-10 (primitives), P2-5, P2-6 (buttons/forms), P2-7, P2-12, P2-19 | M | none |
| B | App shell and navigation | `rtbit-web.tsx`, `components/Sidebar.tsx`, `components/ErrorComponent.tsx`, `components/compact/CategoryFilter.tsx`, `components/compact/TrackerFilter.tsx`, `components/buttons/UploadButton.tsx`, new `hooks/useListOnlyUpload.ts` | P0-1, P1-12, P2-2, P2-13, P2-16 (sidebar), P2-17, P2-20, P3-1 | M | A |
| C | Toolbar and search chrome | `components/Toolbar.tsx`, `components/compact/ActionBar.tsx` | P0-3 (toolbar), P1-1 (toolbar/actionbar), P1-2 (actionbar/toolbar), P1-9 (layout), P1-10 (toolbar), P2-14, P3-4 (search), P3-5, P3-7 | S | A |
| D | Table, cards and context menu | `components/compact/TorrentTable.tsx`, `components/compact/TorrentTableRow.tsx`, `components/compact/PeersTab.tsx`, `components/compact/TableHeader.tsx` (delete), `components/TorrentContextMenu.tsx`, `components/CardLayout.tsx`, `components/TorrentCardContent.tsx` | P0-2, P1-1/P1-2 (CardLayout, ContextMenu), P1-4, P1-5, P1-6, P1-7, P1-8, P2-4, P3-6 (TableHeader) | L (split option below) | A |
| E | Polling and performance | `helper/customSetInterval.ts`, `components/compact/SpeedTab.tsx`, `components/RootContent.tsx`, `main.tsx` | P1-13, P2-1, P2-3, P2-10, P3-8 | M | none (A is preferred) |
| F | Feature-page theming | `components/RSSPage.tsx`, `components/IndexarrBrowse.tsx`, `components/IndexarrSetup.tsx`, `components/LogStream.tsx`, `components/modal/DeleteTorrentModal.tsx`, `components/modal/MultiTorrentUploadModal.tsx` | P1-1 (RSSPage), P2-9 (these files), P2-11, P2-15, P2-16 (RSSPage), P3-2 (RSSPage), P3-3 (RSSPage) | M | A |
| G | Modals, settings and auth | `components/modal/Modal.tsx`, `components/modal/TabbedConfigModal.tsx`, `components/Tabs.tsx`, `components/config/*` (including deleting `AdvancedTab.tsx`), `components/auth/LoginPage.tsx`, `components/auth/SetupPage.tsx` | P1-1 (auth/config), P1-11, P2-8, P2-9 (SecurityTab), P2-16 (RSSTab), P2-18, P3-2 (RSSTab), P3-6 (AdvancedTab) | M | A |
| H | Desktop splash | `desktop/src/index.html` | P2-21, P2-22, P2-23 | S | none |

No file appears in two packages. `hooks/useKeyboardShortcuts.ts`, `stores/*`, `http-api.ts`, `mock-api.ts` and `Footer.tsx` are owned by no package and must not be edited. If a package finds it needs one of them, it should stop and say so in the PR.

---

## WP-A — Tokens and primitives (land first)

**Owns:** `globals.css`, `components/buttons/IconButton.tsx`, `components/buttons/TorrentActions.tsx`, `components/buttons/Button.tsx`, `components/buttons/MagnetInput.tsx`, `components/buttons/FileInput.tsx`, `components/forms/FormCheckbox.tsx`, `components/Spinner.tsx`, `components/ProgressBar.tsx`.

**Steps:**

1. **Contrast (P1-3).** In `globals.css`, set `--color-tertiary` to `slate.500` in light mode (line 21) and `slate.400` in the `.dark` block. Check both values reach 4.5:1 or better on `--color-surface`.
2. **Focus utility (P1-1 foundation).** Add `@utility focus-ring { @apply outline-none focus-visible:ring-2 focus-visible:ring-primary/50 focus-visible:ring-offset-1 ring-offset-surface; }`, or the Tailwind v4 equivalent, to `globals.css`. Document its name in the PR. Packages C, D, F and G apply it.
3. **Coarse-pointer sizing (P1-10).** Add `@utility touch-target { @media (pointer: coarse) { min-width: 44px; min-height: 44px; } }` to `globals.css`.
4. **Real button (P0-4, P2-5).** Rewrite `IconButton.tsx` as a `<button type="button">` that does the following:
   - Takes a required `label: string`, rendered as `aria-label` and `title`.
   - Keeps the optional `href`, but renders an `<a>` only when a real `href` is given.
   - Passes `disabled` through to the element.
   - Maps `color` through a literal lookup (`{ primary: "text-primary", error: "text-error", success: "text-success", ... }`). Remove `` `text-${color}` ``.
   - Replaces `text-blue-500` with `text-primary`.
   - Uses `p-1.5 focus-ring touch-target`.
5. **TorrentActions (P0-3, P2-7).** In `TorrentActions.tsx:78-93`, pass `label="Resume" | "Pause" | "Torrent settings" | "Delete"`. Give each icon a resting semantic colour: resume `text-success`, pause `text-secondary`, settings `text-secondary`, delete `text-error`. Remove the `hover:text-green-600` on the cog.
6. **Blue leak (P2-6).** In `MagnetInput.tsx:36`, `FileInput.tsx:88` and `FormCheckbox.tsx:43`, change `text-blue-*` to `text-primary`, and `group-hover:text-white` to the token equivalent.
7. **Short labels (P1-9).** In `MagnetInput.tsx:37` and `FileInput.tsx:89`, render `<span className="hidden sm:inline">Add Torrent from Magnet / URL</span><span className="sm:hidden">Magnet</span>`. Do the same for "Upload" with "Upload .torrent File". Add `whitespace-nowrap`.
8. **Spinner (P2-19).** Use `role="status"` on the wrapper, keep `aria-hidden` on the SVG, and add `<span className="sr-only">Loading…</span>`. Add `motion-reduce:animate-none` with a static or opacity fallback.
9. **ProgressBar (P2-12).**
   - Render the label as an absolutely positioned overlay across the whole track, with `text-text` on the track and `mix-blend` or a two-layer clip for the filled part, or put the label beside the bar.
   - Fix `now < 1 && "bg-transparent"` so it doesn't emit `false`.
   - Add `role="progressbar" aria-valuenow aria-valuemin=0 aria-valuemax=100`.

**Acceptance:**
- Cards at 390 px show orange or semantic action icons and no blue.
- Tertiary text measures at least 4.5:1 in both themes.
- `IconButton` is announced as a named button, and Space activates it.
- "0.43%" is readable on the card progress bar in both themes.
- The e2e suite passes.

## WP-B — App shell and navigation

**Owns:** `rtbit-web.tsx`, `components/Sidebar.tsx`, `components/ErrorComponent.tsx`, `components/compact/CategoryFilter.tsx`, `components/compact/TrackerFilter.tsx`, `components/buttons/UploadButton.tsx`, and a new `hooks/useListOnlyUpload.ts`.

**Steps:**

1. **Mobile page navigation (P0-1).** In the mobile drawer (`rtbit-web.tsx:254-265`), add a Torrents / Index (when Indexarr is enabled) / RSS segmented list above `<Sidebar />`.
   - It uses `useUIStore` `currentPage` / `setCurrentPage`, the same store calls `Toolbar.tsx:236-265` makes. Read them; don't edit Toolbar.
   - Selecting a page closes the drawer.
   - On a non-torrents page, the filter sections should be hidden.
2. **Drawer fit (P2-13).**
   - Make `Sidebar` accept a `variant="drawer"` that fills its parent (`w-full h-full`) and hides "Collapse".
   - Make the drawer `w-72 max-w-[85vw]`.
   - Add a close button labelled "Close menu", Escape-to-close, and focus moved into the drawer on open.
3. **Engine-unreachable state (P1-12).** When the torrent refresh fails with a network error or a 404/5xx (`rtbit-web.tsx:81`), render a full-panel state from `ErrorComponent.tsx` with:
   - the heading "Can't reach the rustTorrent engine";
   - the API base URL;
   - "Retrying automatically…" with a live region;
   - a Retry button that calls the existing refresh.
   Keep raw HTTP details inside a `<details>`.
4. **Landmarks (P2-17).**
   - Wrap the toolbar slot in `<header>`. The Toolbar component itself is owned by WP-C, so put the landmark on the element that wraps it in `rtbit-web.tsx`.
   - Wrap the content area in `<main>`, the sidebar slot in `<aside aria-label="Filters">` and the footer slot in `<footer>`.
   - Add a visually hidden `<h1>rustTorrent</h1>`.
5. **Heading levels (P2-16).** Make the section headings in `Sidebar.tsx:141`, `CategoryFilter.tsx:98` and `TrackerFilter.tsx:75` `h2`s, keeping their current styles.
6. **Filter rows (P2-20).** Make each status/category/tracker row a `<button aria-pressed>` with a visible `focus-ring`. Expose the count as part of the name, for example `aria-label="Paused, 26 torrents"`.
7. **Stale closure (P2-2).** Remove the `as unknown as` cast at `rtbit-web.tsx:92-98` by giving the store setter the correct type, or wrapping it. Include `API` in the effect deps, or read it through a ref.
8. **Dedupe list-only upload (P3-1).** Extract the logic at `rtbit-web.tsx:126-150` and `UploadButton.tsx:28-45` into `hooks/useListOnlyUpload.ts`, with a cancellation guard, and use it in both places.

**Acceptance:**
- At 390 px, RSS can be reached through the drawer in both themes.
- `10-no-daemon-*` shows the new state.
- axe or Lighthouse reports landmarks and an `h1`.
- The e2e suite passes. Add one e2e test that opens the drawer and navigates to RSS at a mobile viewport.

## WP-C — Toolbar and search chrome

**Owns:** `components/Toolbar.tsx`, `components/compact/ActionBar.tsx`.

**Steps:**

1. **Accessible names (P0-3).** Add `aria-label` to every icon-only button in `Toolbar.tsx:200-391` (resume, pause and delete selected; configure; logs; dark mode; logout; open sidebar) and to both search clear buttons (`Toolbar.tsx:284`, `ActionBar.tsx:243`). Also give the clear buttons `type="button"` (P3-7).
   - The dark-mode toggle gets `aria-pressed`.
   - The dark-mode label says the target state: "Switch to dark theme" or "Switch to light theme".
   - Keep the existing `title`s. The e2e tests use `getByTitle`.
2. **Labelled inputs (P1-2).** Add `aria-label="Filter torrents"` to the toolbar search (`Toolbar.tsx:280`). Add `aria-label`s to the ActionBar search and select (`ActionBar.tsx:220,239`).
3. **Focus (P1-1).** Replace `focus:outline-none` on these inputs with `focus-ring` from WP-A.
4. **Toolbar targets (P1-10).** Add `touch-target` to `toolbarIcon` (`Toolbar.tsx:200-201`). Raise the clear buttons to at least `p-1.5`.
5. **Mobile layout (P1-9, P2-14).**
   - Let the add buttons sit on a single row. Use `flex-nowrap`, `min-w-0`, and an overflow menu below `sm` for the configure/logs/theme cluster if the row still doesn't fit at 390 px.
   - Replace the `w-44` on the search with `w-44 shrink min-w-0`, and replace the ActionBar `w-48` with `flex-1 min-w-0 max-w-48`.
   - The goal is a toolbar no taller than 56 px at 390 px.
6. **Copy (P3-5).** Use "Filter torrents…" as the placeholder in both files. Use verb-first, sentence-case titles: "Settings", "View logs", "Log out".
7. **Radius (P3-4).** Use `rounded-lg` for the search field in both files.

**Acceptance:**
- `01-torrents-mobile-*` has a single-row toolbar.
- The DOM probe reports 0 unnamed buttons in the toolbar.
- The e2e suite passes, with `getByTitle` selectors still matching.

## WP-D — Table, cards and context menu

**Owns:** `components/compact/TorrentTable.tsx`, `components/compact/TorrentTableRow.tsx`, `components/compact/PeersTab.tsx`, `components/compact/TableHeader.tsx` (delete it), `components/TorrentContextMenu.tsx`, `components/CardLayout.tsx`, `components/TorrentCardContent.tsx`.

**Optional split:** this package can be split into **D1** (`TorrentTable.tsx`, `TorrentTableRow.tsx`, `PeersTab.tsx`, `TableHeader.tsx`) and **D2** (`TorrentContextMenu.tsx`, `CardLayout.tsx`, `TorrentCardContent.tsx`). The only interface between them is the existing `onContextMenu(e, torrent)` call shape plus a new optional `anchor?: DOMRect` argument, which D2 adds and D1 consumes.

**Steps:**

1. **Keyboard grid (P0-2), D1.**
   - Give the table `role="grid"` with `aria-rowcount`.
   - Rows get `role="row"`, `aria-selected`, and a roving `tabIndex` (the active row is `0`, the others `-1`).
   - Up/Down move between rows, Home/End jump to the ends, Space toggles selection, Enter opens the detail pane (the same path as a click), and Shift+F10 or the ContextMenu key opens the menu at the row's bounding box.
   - Keep `onMouseDown` behaviour unchanged.
   - Make sure this doesn't conflict with `useKeyboardShortcuts.ts`: read it, and don't edit it.
2. **Sort headers (P1-4), D1.** Put a `<button>` inside each sortable `<th>` (`TorrentTable.tsx:450-456`, `PeersTab.tsx:220`) and set `aria-sort` on the `<th>`.
3. **Column resize (P1-5), D1.**
   - Convert the handle at `TorrentTable.tsx:466-468` to pointer events with `setPointerCapture` and `touch-action: none`.
   - Widen the hit area to 12 px, keeping the visible line at 1.5 px.
   - Add `role="separator" aria-orientation="vertical" tabIndex=0`, with Left/Right arrows resizing by 16 px.
   - Clear the drag state on `pointercancel` and `lostpointercapture`.
4. **Row height (P1-10), D1.** Under `@media (pointer: coarse)`, use `h-11` rows.
5. **Delete `compact/TableHeader.tsx` (P3-6), D1.** Confirm it has no imports first.
6. **Touch path to the menu (P1-6), D2.**
   - Add a "More actions" `IconButton` from WP-A (`label="More actions"`, a `BsThreeDotsVertical` icon) to `TorrentCardContent.tsx`, next to the actions. It opens `TorrentContextMenu` anchored to the button's rect.
   - Add a 500 ms long-press on cards in `CardLayout.tsx` that opens the same menu.
   - Hoist the inline `onContextMenu` closure out of `itemContent` with `useCallback` (P2-4).
7. **Menu semantics (P1-7), D2.**
   - The container gets `role="menu"` and `aria-label`; items get `role="menuitem"`, or `menuitemcheckbox` for Sequential Download.
   - Focus the first item on open, and support Up/Down/Home/End.
   - Escape and Tab close the menu, and focus returns to the element that opened it.
8. **Position (P1-8), D2.** Replace `TorrentContextMenu.tsx:147-150` with a clamp measured after mount: `top = clamp(8, y, innerHeight - menuHeight - 8)`, with the same logic for `left`. Add `max-h-[calc(100dvh-16px)] overflow-y-auto`.
9. **Focus and labels (P1-1, P1-2), D2.**
   - Apply `focus-ring` to the inline inputs in `TorrentContextMenu.tsx:379` and `CardLayout.tsx:128,144,159`.
   - Add `aria-label`s to the CardLayout search ("Filter torrents"), the status select ("Status filter") and the sort select ("Sort by").

**Acceptance:**
- The list can be operated with the keyboard only: Tab into the list, use the arrows, Space, Enter, and Shift+F10.
- At 390 px, the "More actions" button opens the menu fully on screen.
- The menu stays on screen at a 1440×480 viewport.
- The e2e suite passes. Add a keyboard-selection e2e test.

## WP-E — Polling and performance

**Owns:** `helper/customSetInterval.ts`, `components/compact/SpeedTab.tsx`, `components/RootContent.tsx`, `main.tsx`.

**Steps:**

1. **Visibility-aware polling (P1-13).** Inside `customSetInterval.ts`:
   - While `document.hidden` is true, stretch the next delay to `max(delay, 30_000)`, or pause.
   - On `visibilitychange` to visible, cancel the pending timer and run immediately.
   - The exported signature must not change, so the nine call sites benefit without being edited.
   - Add a unit test next to the existing `helper/*.test.mjs` files.
2. **SpeedTab dedupe (P2-1).** Stop the separate 1 s `getTorrentStats` poll at `SpeedTab.tsx:38-58`. Build the history from the torrent's stats in `useTorrentStore`, which is read-only from this package.
3. **Chart colours (P2-10).** Replace the hex values at `SpeedTab.tsx:121,126` with `getComputedStyle(document.body).getPropertyValue("--color-accent-download" / "--color-accent-upload")`, re-read when the theme changes (follow `PiecesCanvas.tsx:66`).
4. **Code-splitting (P2-3).** In `RootContent.tsx`, `React.lazy` the RSS and Indexarr pages behind a `<Suspense fallback={<Spinner/>}>`. Report the bundle size from `vite build` before and after in the PR.
5. **Empty detail pane (P3-8).** If the detail pane is mounted from `RootContent.tsx`, collapse it to a 32 px hint bar while nothing is selected. If the pane is mounted elsewhere (for example `compact/CompactLayout.tsx`, owned by nobody), skip this step and note it in the PR.

**Acceptance:**
- In the DevTools network panel, backgrounding the tab drops request frequency to 30 s or slower.
- The main chunk is smaller.
- The e2e suite passes, including the RSS test, which now goes through the lazy route.

## WP-F — Feature-page theming

**Owns:** `components/RSSPage.tsx`, `components/IndexarrBrowse.tsx`, `components/IndexarrSetup.tsx`, `components/LogStream.tsx`, `components/modal/DeleteTorrentModal.tsx`, `components/modal/MultiTorrentUploadModal.tsx`.

**Steps:**

1. **Status tokens (P2-9).** Replace the raw palette colours with status tokens:
   - `RSSPage.tsx:107,293,606,684-685,765-766`
   - `IndexarrSetup.tsx:166`
   - `IndexarrBrowse.tsx:566,631`
   - `LogStream.tsx:56-62`
   - `DeleteTorrentModal.tsx:82,105` (this also fixes the blue checkbox)
   - `MultiTorrentUploadModal.tsx:93,95,155`

   Use `text-error`/`bg-error-bg/10`, `text-success`, `text-warning`, `text-secondary` and `bg-surface-sunken`, or the nearest existing tokens, and remove every `gray-*`.
2. **Indexarr badges (P2-11).** In `IndexarrBrowse.tsx:47-51`, give every badge a light/dark pair that reaches at least 4.5:1, for example `text-amber-700 dark:text-yellow-400`.
3. **RSS inputs (P1-1, P3-2).** Define one `inputCls` constant at the top of `RSSPage.tsx` that includes `focus-ring`, and use it at lines 93, 279 and 526. Replace `btnSmCls` (line 494) with the WP-A `Button` where it is equivalent.
4. **RSS narrow layout (P2-15).** Below `sm`, render the item, feed and rule lists (`RSSPage.tsx:566,646,727`) as stacked rows (title, then a meta line, then status) instead of a table.
5. **RSS headings (P2-16).** Make the `h3`s at `RSSPage.tsx:101,287` `h2`s, keeping their styles.

**Acceptance:**
- `grep -nE "(gray|blue|red|green|yellow)-[0-9]" <owned files>` returns no hits, or only justified ones listed in the PR.
- `04-rss-*` looks right in both themes and at 390 px. To reach RSS at 390 px before WP-B lands, set the store from the console.

## WP-G — Modals, settings and auth

**Owns:** `components/modal/Modal.tsx`, `components/modal/TabbedConfigModal.tsx`, `components/Tabs.tsx`, `components/config/*` (including deleting `AdvancedTab.tsx`), `components/auth/LoginPage.tsx`, `components/auth/SetupPage.tsx`.

**Steps:**

1. **Scrollable tabs (P1-11).**
   - `Tabs.tsx:33`: make the `TabList` `overflow-x-auto scrollbar-none snap-x` and the tabs `shrink-0 snap-start`.
   - Scroll the active tab into view when it changes.
   - Add `role="tablist"`/`tab`/`tabpanel`, `aria-selected` and Left/Right arrow navigation.
2. **Data colour (P2-8).** In `config/GeneralTab.tsx:9,45`, `config/DHTTab.tsx:9` and `config/ConnectionTab.tsx:9,27,28,80`, change `text-primary` on plain data to `text-text font-medium tabular-nums`. Keep `text-primary` only for links and actions.
3. **Modal name (P2-18).** In `Modal.tsx:46-58`, generate an id with `useId()`, put it on the title `h2` and set `aria-labelledby` on the dialog.
4. **Focus (P1-1).** Use `focus-ring` on `auth/LoginPage.tsx:29`, `auth/SetupPage.tsx:41`, `config/SecurityTab.tsx:59`, `config/RSSTab.tsx:29-30` and `config/ConnectionTab.tsx:80`. In `RSSTab.tsx:30`, replace `w-32` with `w-full max-w-32`.
5. **Tokens (P2-9).** In `config/SecurityTab.tsx:83`, change the raw colours to status tokens.
6. **Headings (P2-16).** In `config/RSSTab.tsx:39,60`, change `h4` to `h3`.
7. **Dead code (P3-6).** Delete `config/AdvancedTab.tsx` after confirming it has no imports.

**Acceptance:**
- In `05-settings-mobile-*`, every tab can be reached by swiping and the active tab is visible.
- Version and Uptime values render in body colour.
- A screen reader announces "Settings, dialog".

## WP-H — Desktop splash

**Owns:** `desktop/src/index.html`.

**Steps:**

1. **Theme and brand (P2-21).** Define CSS custom properties for light and dark under `prefers-color-scheme`, matching the web UI tokens: slate-50 or white surface in light, slate-900 in dark, and an orange-600 or orange-400 accent. Set the spinner's `border-top-color` to the accent. Use "rustTorrent" in the `<title>` and the status text.
2. **Accessibility (P2-22).** Add `role="status" aria-live="polite"` to the status line. Under `@media (prefers-reduced-motion: reduce)`, replace the spin with a slow opacity pulse, or a static ring and an animated ellipsis in the text.
3. **Failure state (P2-23).**
   - After 20 s without a successful `/stats`, change the status to "The rustTorrent engine didn't start", and show a Retry button that restarts polling and a hint to check the log file location.
   - Keep the existing redirect behaviour on success.
   - Don't add a network request to any host other than `127.0.0.1:3030`.

**Acceptance:**
- The splash renders in both schemes. Open the file directly in a browser and toggle the emulated scheme.
- After 20 s with no engine, the failure state appears.
- `cargo tauri build` isn't required for review. If you do run it, attach a screenshot.

---

## After phase 2

Phase 3 re-runs the audit capture (same viewports, schemes and screenshot names under `design/audits/<date>/`) and checks each finding ID against these packages. If a package leaves a finding open, list its ID in that package's PR under "Not done", with the reason.
