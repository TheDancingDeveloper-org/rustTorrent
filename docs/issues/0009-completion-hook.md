# Issue #9 — Run action/script on download or queue completion

**Crate:** `librtbit` (session) + config/WebUI · **Effort:** M

## Problem

Provide a generalized completion hook: when a torrent finishes, execute an
external command with torrent metadata exposed as environment variables
(qBittorrent 5.2.0 "run on completion", broadened to download **or** queue
completion).

## Current state (file:line)

The completion-watcher pattern already exists and is the natural hook point:

- Download-finished detection: `librtbit/src/torrent_state/live/mod.rs:682-687`
  (`on_piece_completed` → `chunks.is_finished()` → `finished_notify.notify_waiters()`).
- Completion watcher: `librtbit/src/session/mod.rs:1146-1189`
  (`spawn_completion_watcher`) awaits `handle.wait_until_completed()`
  (`torrent_state/mod.rs:612-634`), then currently only moves files to
  `completed_folder` (`:1180-1188`).
- Metadata available on the handle at that point:
  - name `handle.name()`; info-hash `handle.info_hash()` /
    `handle.shared().info_hash.as_string()`;
  - category `handle.shared().category.read()` (`session/mod.rs:1174`);
  - save path `handle.shared().options.output_folder` (`:1151`);
  - total size `handle.metadata.load()…info.lengths().total_bytes()`;
  - stats (uploaded/downloaded/ratio) `handle.stats()`
    (`torrent_state/stats.rs:70-83`).

## Proposed implementation

### Phase 1 — config

Add to `SessionOptions` (`librtbit/src/session/types.rs:250-328`) and the
persisted `SessionSettings`:

```text
completion_command: Option<String>   // shell command or program path
completion_args: Vec<String>         // optional, default []
run_on_queue_complete: bool          // also fire when the whole queue drains
```

Persist via the existing settings mechanism (`session/mod.rs` ~`:152`).

### Phase 2 — execute on completion

In `spawn_completion_watcher` (`session/mod.rs:1146-1189`), after (or alongside)
the move-to-completed step, spawn the command with `tokio::process::Command`:

- Run **after** the move so `RT_SAVE_PATH` reflects the final location.
- Don't block the watcher: spawn detached, log non-zero exit, capture stdout/stderr
  to the tracing log (truncated).
- **Env vars** (prefix `RT_`, qBittorrent-style names also accepted as aliases):
  `RT_TORRENT_NAME`, `RT_INFOHASH_V1`, `RT_SAVE_PATH`, `RT_CONTENT_PATH`,
  `RT_CATEGORY`, `RT_TAGS`, `RT_SIZE_BYTES`, `RT_NUM_FILES`, `RT_RATIO`,
  `RT_UPLOADED_BYTES`, `RT_DOWNLOADED_BYTES`, `RT_TRACKER`.
- Security: do **not** pass the command through a shell by default (avoid
  injection from torrent names); exec the program with explicit args. If a shell
  string is desired, make it an explicit opt-in and document the risk.

### Phase 3 — queue completion (optional broadening)

Track outstanding active downloads; when the count transitions to zero, fire an
optional second hook (`run_on_queue_complete`). Keep separate from per-torrent so
both can be configured independently.

### Phase 4 — surface

- qBit-compat API fields for `autorun` if present in the compat surface, plus
  native config endpoint.
- WebUI settings field (command + args + enable toggles).

## Testing

- Unit: env-var map built correctly from a fixture handle/stats.
- Integration: completion fires a recorded script (writes env to a temp file);
  assert vars + that a torrent name with shell metacharacters does **not** execute
  injected content (no-shell exec).
- Failure: non-zero exit logged, watcher continues, move still completes.

## Risks / notes

- Command injection via torrent-controlled fields — exec without shell.
- Long-running / hanging user scripts: run detached with a timeout; never block
  the session task.
- Shared crate → coordinate publish with StackArr.
