# Releasing

A release starts with a tag. Everything after the tag is automatic: the CI
builds one binary for each platform, makes the GitHub release, and sends the
crate to crates.io.

## Once, before the first release

1. Make the repository public. The installer and the updater download the
   binaries from the GitHub release without a login. The build of the release
   gets the Git LFS fonts without a login. Neither works in a private
   repository.
2. Write one secret in the repository, at Settings, Secrets and variables,
   Actions:

   | Secret | Where it comes from |
   | ------ | ------------------- |
   | `CARGO_REGISTRY_TOKEN` | crates.io, at Account Settings, API Tokens, with the scope `publish-new` for the first release and `publish-update` after it |

   `GITHUB_TOKEN` needs no work, because GitHub gives it to each workflow.

## The steps

1. Move the facts of the release into the changelog. In `CHANGELOG.md`, the
   heading `## [Unreleased]` becomes the version and the day:

   ```markdown
   ## [0.2.0] - 2026-08-14
   ```

   Then write a new empty `## [Unreleased]` above it. The release notes on
   GitHub are this section, so what it does not say, the release does not say.

2. Write the same version in `Cargo.toml`. One command does it, and writes
   `Cargo.lock`:

   ```bash
   cargo install cargo-edit    # once
   cargo set-version 0.2.0
   ```

3. Run what each change runs:

   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

4. Read what goes to crates.io, without sending it:

   ```bash
   cargo publish --dry-run --locked
   ```

5. Commit, tag and push. The tag is the version with a `v` in front of it:

   ```bash
   git commit -am "release: 0.2.0"
   git tag v0.2.0
   git push && git push --tags
   ```

   Push the commit first and let the CI pass. `publish-crates.yml` does not
   publish a commit that has no green CI.

The number itself follows [Semantic Versioning](https://semver.org). A change
that makes an old command or tool answer in a new way is a major release, even
when the code of the change is small. Before version 1.0.0, a minor release
can hold such a change.

## What the tag starts

| Workflow | What it does |
| -------- | ------------ |
| `release.yml` | Plans the release, builds each platform on its own runner, and makes the GitHub release with the archives, the checksums and the installer. |
| `publish-crates.yml` | Waits for that release. Then it checks that the tag and `Cargo.toml` have the same version, and that the checks of `ci.yml` passed for the tagged commit. Then it sends the crate to crates.io. |

`publish-crates.yml` runs after the release exists, so a failure at crates.io
leaves the binaries where they are. To send the crate again after such a
failure, start `Publish to crates.io` by hand from the Actions page and give it
the tag:

```bash
gh workflow run publish-crates.yml -f tag=v0.2.0
```

A crate on crates.io is permanent. A version that went out cannot go out again
with different contents, so step 4 is the step to do carefully.

## The releases

The releases hold `sliderino` for Linux (x86_64) and for macOS (Apple silicon).
`sliderino-debug` is a cargo example, so it is not in the archives and not in
`cargo install`.

The Linux binary is built on Ubuntu 24.04. It needs glibc 2.39 or newer. Both
binaries need GStreamer on the machine of the user. The README says how to
install it. The macOS binary is linked to the GStreamer of Homebrew.

## The configuration of the release

`dist-workspace.toml` holds the platforms, the installer and the tools that
each runner installs. `.github/build-setup.yml` holds the steps that run in the
build job before the build: the libraries and the Git LFS fonts.
`.github/workflows/release.yml` comes from `dist-workspace.toml`, so no hand
edits go in it. After a change:

```bash
dist generate
git add dist-workspace.toml .github
```

To read what a release would hold, without a build and without a tag:

```bash
dist plan
```

To make the installer on this machine, which is how to read what it does:

```bash
dist build --artifacts=global
```

To build the archive of this machine, which takes as long as one runner takes:

```bash
dist build --artifacts=local
```

Each of the three writes to `target/distrib`.

## A newer dist

`cargo-dist-version` in `dist-workspace.toml` says which version of `dist` the
CI uses. To move to a newer one, install it and let it write the file again:

```bash
cargo install cargo-dist --locked
dist init
dist generate
```

Read the difference in `release.yml` before the commit. That file decides which
runner builds each platform, and a new version of `dist` can move a build to
another image of the operating system.

## A release that must not go out yet

A tag such as `v0.2.0-rc.1` makes a pre-release on GitHub. `dist` marks it as
one, so the address `releases/latest/download/...` still gives the version
before it, and the installer of a user gives the stable release.
`publish-crates.yml` skips a pre-release, so crates.io does not get it.
