# Issue #12 — BEP 46 Phase 2: DHT mutable-item lookup for updateable magnets

**Crate:** `librtbit-dht` (+ `librtbit` session integration) · **Effort:** L

## Problem

BEP 46 lets a magnet point at a public key (`xs=urn:btpk:<hex>`, optional
`s=<salt>`) instead of an info-hash; the current info-hash is resolved by fetching
a **BEP 44 mutable item** from the DHT. The crypto and storage primitives exist,
but the DHT get/put are stubs and nothing resolves a BEP 46 magnet.

## Current state (file:line)

Done:
- Ed25519 sign/verify + target derivation: `librtbit-dht/src/bep44_crypto.rs:1-112`.
- `MutableItemStore` (in-memory, LRU-ish): `librtbit-dht/src/mutable_item_store.rs`
  — `new` `:40`, `store` `:54`, `get` `:72`, `StoredMutableItem {k,sig,seq,v,salt,
  last_updated}` `:13-27`. **Instantiated but not referenced by the `Dht` struct.**
- Magnet parsing: `librtbit-core/src/magnet.rs` — `public_key: Option<[u8;32]>`
  `:15` (parsed from `xs=urn:btpk:` `:123-134`, accessor `as_public_key` `:39`);
  `salt: Option<String>` `:17` (parsed from `s=` `:136-139`, accessor `as_salt`
  `:44`).

Missing / stubbed:
- Incoming BEP44 GET handler: `librtbit-dht/src/dht.rs:928-958` — returns closest
  nodes + token only; does **not** consult `MutableItemStore`.
- Incoming BEP44 PUT handler: `dht.rs:960-984` — empty response; no signature
  verification, no storage.
- **No outbound `get_mutable` iterative lookup** and no client API to resolve a
  target → item.
- Magnet resolution ignores BEP46: `session/mod.rs:625-653` parses the magnet
  (`:631`) then only extracts `info_hash` via `as_id20()` (`:633-635`); no branch
  for `as_public_key().is_some()`. `peer_sources.rs` handles only v1/v2 hashes.

Existing iterative-lookup machinery to mirror (`dht.rs`):
- `pub fn get_peers()` `:1437-1442` → `RequestPeersStream`.
- `RecursiveRequest` `:262-486`; `get_peers_root` seeds 8 closest `:393-415`;
  `request_one` loop (contact node, parse `nodes`/`nodes6`, requeue closer)
  `:419-486`.

## Proposed implementation

### Phase 1 — wire `MutableItemStore` into the node (incoming side)

- Give the `Dht` struct an `Arc<MutableItemStore>`.
- GET handler (`dht.rs:928-958`): if the store has the target, include the item
  (`k,seq,sig,v`, and `salt` echo) in the response alongside nodes+token.
- PUT handler (`dht.rs:960-984`): validate the write token, then
  `bep44_crypto::verify_mutable_item(...)`; enforce monotonic `seq` (reject
  `seq` ≤ stored); on success `mutable_item_store.store(target, item)`. Implements
  BEP 44 §"Mutable items" storage rules (cas, seq, salt ≤ 64 bytes).

### Phase 2 — outbound iterative `get_mutable`

- Add a `RecursiveRequest` variant (mirror `get_peers_root` / `request_one`
  `:393-486`) that issues `get` for a mutable target and collects any returned
  `item`. Seed from nodes closest to `target = bep44_crypto::mutable_item_target(
  pubkey, salt)`.
- On each response carrying an item: verify signature against the requested public
  key, track the **highest valid `seq`** (BEP 46 says take the freshest), and keep
  querying closer nodes per the existing loop.
- Expose `pub async fn get_mutable(target, pubkey, salt) -> Option<MutableItem>`
  (and optionally a `put_mutable` for republishing if rustTorrent ever authors
  mutable torrents — out of scope for resolve-only).

### Phase 3 — session resolution of BEP 46 magnets

In `session/mod.rs:625-653`, after parsing (`:631`) and **before** the
`as_id20()` extraction (`:633`):

```text
if let Some(pk) = magnet.as_public_key() {
    target = mutable_item_target(pk, magnet.as_salt());
    item   = dht.get_mutable(target, pk, salt).await?;   // freshest valid seq
    // item.v is the bencoded value; per BEP 46 it carries the current info-hash
    info_hash = parse_infohash_from_value(item.v)?;
    // then continue the normal resolve_magnet(info_hash) path
}
```

- Decode `item.v` to obtain the current info-hash (BEP 46 stores the target's
  current info-hash in the mutable value), then fall through to the existing
  magnet→metadata resolution.
- Optional: keep the public key on the torrent and periodically re-query to follow
  updates (the "updateable" half of "updateable magnets"); minimum viable is
  resolve-at-add.

## Testing

- Unit: PUT handler accepts a correctly signed item, rejects bad signature, rejects
  non-monotonic `seq`, enforces salt length; GET returns a stored item.
- Unit: target derivation matches `bep44_crypto::mutable_item_target` for
  salt/no-salt (BEP 44 test vectors).
- Integration: two in-process DHT nodes — PUT on A, `get_mutable` from B returns the
  item; freshest-`seq` wins when nodes disagree.
- Integration: a `xs=urn:btpk:` magnet resolves to an info-hash and proceeds to
  metadata fetch.

## Risks / notes

- Signature verification is mandatory on both store and resolve — never trust an
  unverified item.
- Monotonic `seq` + freshest-wins prevents rollback attacks.
- Salt ≤ 64 bytes; reuse the existing parser's salt field.
- Shared crates `librtbit-dht` + `librtbit-core` → coordinate publish (and the
  dependency matrix) with StackArr.
