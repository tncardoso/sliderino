# Sliderino

## Goal

- Fast, responsive interface.
- Fast startup times.

## Instructions

- Run lint, format and tests at every code change
    - `cargo clippy`, `cargo fmt`, `cargo test`
- Only run one cargo tool at a time (clippy, fmt, build, test)
- When fixing bugs, add regression tests
- Documentation should be written in ASD-STE100 simplified technical english
- Keep documentation up to date with changes
- Create git commits with Conventional Commits specification
- Record each change a user sees in `CHANGELOG.md`, under `## [Unreleased]`
    - Put the line in an `### Added`, `### Changed`, `### Fixed`,
      `### Deprecated`, `### Removed` or `### Security` subsection
    - The release notes are that text, so write for a user of aphid and not
      for a reader of the code
    - Do not write a version heading or a date. A release does that
    - A change that only touches the build, the tests or the internals needs
      no line

