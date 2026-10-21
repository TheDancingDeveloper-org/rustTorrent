# Migrating torrents from qBittorrent

`scripts/import-qbit` moves the torrent list from a qBittorrent `BT_backup`
directory into rustTorrent through its WebUI API v2 compatibility layer. It
uses only the Python 3 standard library.

It reads each `<hash>.torrent` and `<hash>.fastresume` pair, then:

1. creates any categories the backup uses that rustTorrent does not have yet;
2. adds each torrent **stopped**, with the same save path, category and tags;
3. skips info-hashes rustTorrent already has, so a re-run is safe;
4. with `--recheck`, queues a recheck for the added torrents in throttled
   batches (`--recheck-batch`, default 10, every `--recheck-delay` seconds,
   default 30).

The tool never opens, moves or writes payload files. rustTorrent must see the
payload at the same paths qBittorrent used (for example, mount the same
`/qbtdownloads` volume into the rustTorrent container).

Ratio and seeding-time limits are not migrated; the report counts how many
torrents had custom limits so you can set them by hand.

## Dry run first

The dry run is the default. It contacts rustTorrent read-only (torrent list and
categories) and prints a JSON report:

```sh
export RUSTTORRENT_URL=http://127.0.0.1:3030
export RUSTTORRENT_USERNAME=admin
read -rs RUSTTORRENT_PASSWORD && export RUSTTORRENT_PASSWORD
scripts/import-qbit /path/to/qBittorrent/BT_backup
```

The report shows `total`, `status` (`new` / `already-present`), parse
`errors` (for example a `.fastresume` without its `.torrent`), the categories
in use, `missing_categories`, save-path roots and tag/limit counts. Add
`--details` for one row per torrent, or `--offline` to parse the backup
without contacting rustTorrent.

## Apply

```sh
scripts/import-qbit /path/to/BT_backup --apply            # add, stopped
scripts/import-qbit /path/to/BT_backup --apply --recheck  # and verify data
```

The exit code is 0 on success, 1 if any add failed (listed under
`result.failed`), and 2 if rustTorrent could not be reached or rejected the
login. Start the torrents from the UI once the rechecks finish.

## Credentials

Credentials are never accepted on the command line. Use
`RUSTTORRENT_USERNAME` / `RUSTTORRENT_PASSWORD`, or `--credentials-file`
pointing at a `0600` file with `username=` and `password=` lines (environment
variables win). With no username the tool does not log in, which works when
the WebUI has no credentials set.

## Tests

`python3 scripts/test_import_qbit.py` runs the tool against a fake API built
on `http.server`. CI runs it in the `scripts` job.
