# Contributing to allcast

## Branch model

- `dev` — integration branch. **All changes land here via pull request.**
- `master` — release branch. Promoted from `dev` via pull request. Direct pushes are blocked.

```
feature/* ──PR──▶ dev ──PR──▶ master ──▶ tagged release + notes
```

## Pull requests

1. Branch off `dev`: `git checkout dev && git pull && git checkout -b feature/my-change`.
2. Open a PR into `dev`. CI must pass (`cargo build`, `cargo test`, `cargo clippy -- -D warnings`).
3. Releases are cut by promoting `dev` into `master` via PR.

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
