# CI & Release Workflows — Design

**Date:** 2026-06-03
**Repo:** github.com/aubrey-silvey/allcast (Rust workspace: `allcast`, `allcast-sender`, `allcast-receiver`)
**Status:** Approved for planning

## Goal

Establish a branch model and GitHub Actions workflows so that:

1. All work reaches the codebase through pull requests into the trunk (`master`).
2. The trunk (`master`) produces tagged releases.
3. Release notes / `CHANGELOG.md` are generated automatically from commit history on `master`.

## Decisions

| Decision | Choice |
|---|---|
| Branch model | Trunk-based: `master` is the single long-lived branch; all changes via PR, direct pushes blocked (standard open-source flow) |
| Release tooling | [release-please](https://github.com/googleapis/release-please-action) |
| Commit convention | Conventional Commits, **documented only** (no CI enforcement) |
| Versioning | Single shared version across all 3 crates, one tag, one CHANGELOG |
| Branch protection | Applied by us via `gh api` (admin auth required) |
| CI checks | `cargo build`, `cargo test`, `cargo clippy -- -D warnings` |
| Out of scope (YAGNI) | Compiled-binary release artifacts; crates.io publishing |

## Branch Model & Flow

- `master` — the single long-lived trunk. Contributors fork (or branch off `master`) and open PRs
  targeting `master`. CI gates every PR; direct pushes are blocked. release-please watches `master`.

```
fork / feature/* ──PR──▶ master ──▶ release-please ──▶ tag vX.Y.Z + GitHub Release notes
                  (CI gates)        (Release PR: CHANGELOG.md + version bumps)
```

release-please operates on `master`: as Conventional Commits land there, it opens/maintains a
"Release PR" that updates `CHANGELOG.md` and bumps the three crate versions in lockstep. Merging
that Release PR creates the git tag `vX.Y.Z` and a GitHub Release whose body is the generated notes.

## Components

### 1. `.github/workflows/ci.yml`

- **Triggers:** `pull_request` targeting `master`; `push` to `master`.
- **Runner:** `ubuntu-latest`.
- **System deps step** (required — crates link system GStreamer; `eframe` needs X11/Wayland libs):
  ```
  sudo apt-get update
  sudo apt-get install -y \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    libx11-dev libxext-dev libwayland-dev libxkbcommon-dev libgl1-mesa-dev
  ```
- **Steps:** checkout → install system deps → `dtolnay/rust-toolchain@stable` (with `clippy`) →
  `Swatinem/rust-cache` → `cargo build --workspace --locked` → `cargo test --workspace --locked` →
  `cargo clippy --workspace --all-targets -- -D warnings`.
- These three jobs/steps are the **required status checks** for the `master` branch protection rule.

### 2. `.github/workflows/release-please.yml`

- **Trigger:** `push` to `master`.
- **Permissions:** `contents: write`, `pull-requests: write`.
- **Action:** `googleapis/release-please-action@v4` with `config-file: release-please-config.json`
  and `manifest-file: .release-please-manifest.json`.
- **Auth:** default `GITHUB_TOKEN`.
- **Result:** maintains the Release PR; on merge, creates tag `vX.Y.Z` + GitHub Release with notes.

### 3. `release-please-config.json`

- `release-type: rust` for each of the three packages (`allcast`, `sender`, `receiver`).
- `plugins`: `cargo-workspace` (keeps member versions + `Cargo.lock` in sync) and `linked-versions`
  (groups all crates under one version and a single tag).
- Single root `CHANGELOG.md`, single tag format `v$VERSION` (no per-crate component prefix).

### 4. `.release-please-manifest.json`

- Seeds every package path at the current version `0.1.0`.

### 5. `CONTRIBUTING.md` (commit convention doc)

- Documents the branch flow (fork/branch → PR → `master`).
- Documents Conventional Commit prefixes (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, etc.)
  and that they drive the changelog and version bump (`feat` → minor, `fix` → patch,
  `feat!`/`BREAKING CHANGE` → major).

### 6. Branch protection (applied via `gh api`, not committed files)

- `master`: require a pull request before merging (blocks direct pushes), and require the CI status
  check (`build / test / clippy`) to pass before merge.
- Requires repo **admin** to apply. Recorded as exact `gh api` commands in the implementation plan.

## Error Handling / Edge Cases

- **CI build failure from missing system libs:** mitigated by the explicit apt step; pinned package
  names verified against `ubuntu-latest`.
- **`--locked` drift:** builds use `--locked` so a stale `Cargo.lock` fails fast rather than silently
  resolving new versions.
- **Non-conventional commits on `master`:** release-please ignores commits it can't classify; they
  won't break the workflow but won't appear in notes. Documented, not enforced.
- **First release:** with the manifest seeded at `0.1.0`, the first qualifying commit produces the
  first Release PR; the initial tag will be the next bump (e.g. `v0.1.1` or `v0.2.0`).

## Testing / Verification

- Open a PR into `master` to confirm CI triggers and all three checks run.
- Land a `feat:`/`fix:` commit on `master` and confirm release-please opens a Release PR with a
  populated `CHANGELOG.md` and synced crate versions.
- Confirm the branch-protection rule rejects a direct push to `master`.

## Files Touched

- `.github/workflows/ci.yml` (new)
- `.github/workflows/release-please.yml` (new)
- `release-please-config.json` (new)
- `.release-please-manifest.json` (new)
- `CONTRIBUTING.md` (new)
- Branch protection rule on `master` (via `gh api`)
