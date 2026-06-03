# Contributing to allcast

## Branch model

We use a trunk-based flow: `master` is the single long-lived branch. All changes land via
pull request, and direct pushes to `master` are blocked.

```
fork / feature/* ──PR──▶ master ──▶ tagged release + notes
```

## Pull requests

1. Fork the repo (external contributors) or branch off `master`:
   `git checkout master && git pull && git checkout -b feature/my-change`.
2. Open a PR targeting `master`. CI must pass (`cargo build`, `cargo test`,
   `cargo clippy -- -D warnings`).
3. Once merged, [release-please](https://github.com/googleapis/release-please) maintains a
   "release" PR; merging it tags the release and publishes notes — no manual release steps.

## Commit messages — Conventional Commits

Release notes and version bumps are generated automatically from commit messages on `master`,
so commits **must** follow [Conventional Commits](https://www.conventionalcommits.org/):

| Prefix | Meaning | Version effect |
|---|---|---|
| `feat:` | New feature | minor bump |
| `fix:` | Bug fix | patch bump |
| `docs:` | Documentation only | none |
| `chore:` / `ci:` / `refactor:` / `test:` | Maintenance | none |
| `feat!:` or a `BREAKING CHANGE:` footer | Breaking change | major bump |

Example: `feat(sender): add hardware H.264 encode path`

All three crates share a single version and are released together under one tag (`vX.Y.Z`).
