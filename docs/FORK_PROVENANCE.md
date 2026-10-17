# Fork provenance

rustTorrent is a permanent hard fork of Igor Katson's rqbit project. The
original Apache-2.0 copyright and license notice remains in place; this
repository does not merge future upstream changes automatically.

The shared crates were consolidated into this monorepo from these Forgejo
`main` commits on 2026-07-12:

| Crate | Import commit |
|---|---|
| `librtbit` | `ebe3b08` |
| `librtbit-bencode` | `6dcfe4e` |
| `librtbit-buffers` | `419c931` |
| `librtbit-clone-to-owned` | `c4695ad` |
| `librtbit-core` | `03d397b` |
| `librtbit-dht` | `cc70c2c` |
| `librtbit-lsd` | `1891e65` |
| `librtbit-peer-protocol` | `3204753` |
| `librtbit-sha1-wrapper` | `01328fc` |
| `librtbit-tracker-comms` | `ba038a6` |
| `librtbit-upnp` | `8d8c10e` |
| `librtbit-upnp-serve` | `88dddc1` |

Upstream advisories and changelogs may be monitored for security ideas to
hand-port, but upstream source is not an integration dependency.
