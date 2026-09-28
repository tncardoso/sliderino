---
name: release
description: Cut a new version release of sliderino. Use when the user asks to release, cut a version, bump the version, tag a release, or publish to crates.io. Covers the changelog, the version bump, the checks and the tag that starts the CI.
---

# Release

A release starts with a tag. Everything after the tag is automatic: the CI
builds one binary for each platform, makes the GitHub release from the
changelog, and sends the crate to crates.io.

The full reference is `docs/releasing.md`. This skill is the procedure.

## Before you start

1. Ask the user for the version number if they did not give one. The number
   follows [Semantic Versioning](https://semver.org). A change that makes an
   old command or tool answer in a new way is a major release, even when the
   code of the change is small.
2. For the first release only: check with `gh repo view --json visibility`
   that the repository is public, and ask the user if the secret
   `CARGO_REGISTRY_TOKEN` is set. If one is missing, stop and tell the user.
   The steps are in `docs/releasing.md`.
3. Be on `main`, up to date with the remote, with a clean working tree:

   ```bash
   git switch main && git pull
   git status --short
   ```

4. Read `## [Unreleased]` in `CHANGELOG.md`. If it is empty, there is nothing
   to release: stop and tell the user.

In the steps below, `X.Y.Z` is the new version and `P.Q.R` is the version
before it.

## The steps

### 1. Move the facts of the release into the changelog

In `CHANGELOG.md`, the heading `## [Unreleased]` becomes the version and the
day. Use today's date, in `YYYY-MM-DD`:

```markdown
## [X.Y.Z] - 2026-08-14
```

Then write a new empty `## [Unreleased]` above it.

At the end of the file, keep the link references. The first release adds
them. The `Unreleased` link
compares from the new version, and the new version gets a link of its own:

```markdown
[Unreleased]: https://github.com/tncardoso/sliderino/compare/vX.Y.Z...main
[X.Y.Z]: https://github.com/tncardoso/sliderino/compare/vP.Q.R...vX.Y.Z
[P.Q.R]: https://github.com/tncardoso/sliderino/releases/tag/vP.Q.R
```

The link of the first release is a `releases/tag/` link, because there is no
version before it to compare with.

The release notes on GitHub are this section, so what it does not say, the
release does not say. Write for a user of Sliderino and not for a reader of the
code, in ASD-STE100 simplified technical English.

### 2. Write the same version in `Cargo.toml`

One command does it, and writes `Cargo.lock`:

```bash
cargo install cargo-edit    # once, if `cargo set-version` is missing
cargo set-version X.Y.Z
```

### 3. Run what each change runs

Run one cargo tool at a time:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

All three must pass. If one fails, fix it before you go on, or stop and tell
the user.

### 4. Read what goes to crates.io, without sending it

```bash
cargo publish --dry-run --locked
```

A crate on crates.io is permanent. A version that went out cannot go out again
with different contents, so this is the step to do carefully.

### 5. Commit, tag and push

The tag is the version with a `v` in front of it:

```bash
git commit -am "release: X.Y.Z"
git push
```

Wait until the CI of that commit is green (`gh run watch`). Then:

```bash
git tag vX.Y.Z
git push --tags
```

Ask the user before you push. The push starts the release, and a tag that is
public cannot be taken back cleanly.

## What the tag starts

| Workflow | What it does |
| -------- | ------------ |
| `release.yml` | Plans the release, builds each platform on its own runner, and makes the GitHub release with the archives, the checksums and the installer. |
| `publish-crates.yml` | Waits for that release. It checks the tag, the version and the CI of the commit. Then it sends the crate to crates.io. |

To watch them:

```bash
gh run list --limit 5
```

`publish-crates.yml` runs after the release exists, so a failure at crates.io
leaves the binaries where they are. To send the crate again after such a
failure, start `Publish to crates.io` by hand from the Actions page and give it
the tag:

```bash
gh workflow run publish-crates.yml -f tag=vX.Y.Z
```

## Rules

- Do not edit `.github/workflows/release.yml`. `dist generate` writes it from
  `dist-workspace.toml`. After a change to that file:

  ```bash
  dist generate
  git add dist-workspace.toml .github
  ```

- Do not write the version by hand in `Cargo.toml`. `cargo set-version` writes
  it and `Cargo.lock` as well.
- `sliderino-debug` is a cargo example on purpose: `dist` ships each binary of
  the package, and it does not read `required-features`.

## To read a release before there is one

```bash
dist plan                          # what the release would hold
dist build --artifacts=global      # the installer, on this machine
dist build --artifacts=local       # the archive of this machine
```

Each of the three writes to `target/distrib`.
