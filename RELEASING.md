# Releasing rustTorrent

GitHub is the release authority for rustTorrent. Release tags use the `vX.Y.Z`
format, or `vX.Y.Z-{alpha,beta,rc}.N` for a pre-release, and must point to a
commit reachable from `main`.

## Release channels

The container image is published to GHCR (`ghcr.io/thedancingdeveloper-org/rusttorrent`)
on these tags:

- **`:latest`**: the current stable release. Moved only by a stable `vX.Y.Z`
  tag, including a stable tag pushed directly as a hotfix.
- **`:beta`**: the newest pre-release, for soak testing. A pre-release tag
  **never** moves `:latest`.
- **`:vX.Y.Z` / `:vX.Y.Z-rc.N`** (and `-alpha.N`, `-beta.N`): immutable, one per tag.
- **`:sha-<commit>`**: immutable, one per build.

Pre-release tags are flagged as GitHub pre-releases. A pre-release tag publishes
only `:<tag>` and `:beta` for the image, and a GitHub pre-release with binaries.
The download host's `latest` link moves only for stable tags.

## Beta soak and promotion

Ship a stable release only after a pre-release has soaked:

1. Tag a pre-release on a `main` commit, e.g. `v0.1.0-rc.1`, and push the tag.
   `container.yml` publishes `:v0.1.0-rc.1` and `:beta`. `release.yml` publishes
   the binaries and a GitHub pre-release. `:latest` is untouched.
2. Let it soak. Cut a new suffix (`-rc.2`, …) for fixes found during soak.
3. When satisfied, run the **Promote pre-release to stable** workflow
   (`workflow_dispatch`, `beta_tag` = the soaked tag, e.g. `v0.1.0-rc.1`). It:
   - resolves the stable version by dropping the suffix (`v0.1.0`) and checks
     that the soaked tag is reachable from `main`;
   - copies the **exact image digest** of `:<beta_tag>` to `:vX.Y.Z` and
     `:latest`, and verifies both destination digests match it;
   - creates the stable tag `vX.Y.Z` on the same commit (it is idempotent, and
     it fails if that tag already points somewhere else);
   - calls `release.yml` for the stable tag, which builds, signs and publishes
     the binaries and the stable GitHub release.

Because promotion copies the soaked image by digest, `:vX.Y.Z` is byte-identical
to the pre-release that was tested. Only the tag changes.

The stable tag is created with the workflow token, which does not trigger
`container.yml` or `release.yml`. The stable binaries come from the explicit
`release.yml` call, not from a second push.

A stable `vX.Y.Z` tag pushed directly (a hotfix without soak) still works. It
publishes `:vX.Y.Z` and `:latest` and runs `release.yml` as a normal tag push.

### Version metadata

The binaries report the version in `crates/rtbit/Cargo.toml`, as the commit
stands at the soaked tag. Promotion rebuilds from that same commit, so the
promoted binaries carry whatever version the soaked commit declares. Set the
crate version to the stable version before cutting the pre-release if the
promoted binaries must read `X.Y.Z`. The image's embedded version is not
changed by promotion, because the image is copied, not rebuilt.

## Release checklist

1. Set the crate version in `crates/rtbit/Cargo.toml` and any user-visible
   version metadata.
2. Run the checks in `docs/TESTING.md` and the `Makefile`.
3. Merge the release change to `main` through a reviewed pull request.
4. Push an annotated pre-release tag (`vX.Y.Z-rc.N`) on that `main` commit.
5. Monitor `container.yml` and `release.yml` on the tag.
6. Soak, then run **Promote pre-release to stable** with the tag.
7. Verify the GitHub release assets, checksums and signatures, and the
   multi-architecture GHCR image:

   ```text
   ghcr.io/thedancingdeveloper-org/rusttorrent:vX.Y.Z
   ghcr.io/thedancingdeveloper-org/rusttorrent:latest
   ```

Release notes: a tag with a checked-in `RELEASE_NOTES_<tag>.md` uses that file
verbatim. A tag without one, such as a promoted stable tag, gets
GitHub-generated notes.

## Rollback

Do not move or replace a published tag. Revert the faulty change on `main`,
release a new patch version, and publish a new tag. Operators who need a
controlled rollout should pin an immutable image tag (`:vX.Y.Z` or
`:sha-<commit>`) rather than `:latest` or `:beta`.
