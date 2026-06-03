# CI & Release Workflows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a trunk-based (`master`-only) branch model with GitHub Actions for CI and release-please-driven tagged releases + changelog.

**Architecture:** Contributors fork or branch off `master` and open PRs targeting `master` (gated by a CI workflow running build/test/clippy with system GStreamer + GUI libs). release-please watches `master`, maintains a Release PR (CHANGELOG.md + lockstep version bumps across the 3 crates), and on merge creates tag `vX.Y.Z` + a GitHub Release with generated notes. Branch protection on `master` is applied via `gh api` (admin required).

**Tech Stack:** GitHub Actions, `googleapis/release-please-action@v4`, `dtolnay/rust-toolchain`, `Swatinem/rust-cache`, Rust/Cargo workspace, `gh` CLI.

**Spec:** `docs/superpowers/specs/2026-06-03-ci-release-workflows-design.md`

---

## File Structure

| File | Responsibility |
|---|---|
| `.github/workflows/ci.yml` | PR/push CI: system deps + build/test/clippy |
| `.github/workflows/release-please.yml` | Run release-please on push to `master` |
| `release-please-config.json` | Rust workspace release config (3 crates, linked single version) |
| `.release-please-manifest.json` | Tracks current version per package (seed `0.1.0`) |
| `CONTRIBUTING.md` | Branch flow + Conventional Commit convention docs |

All work happens on a branch off `master`; the workflow files must be merged into `master` (via PR) before their triggers fire on the intended branches.

---

### Task 1: Add CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Write the CI workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  pull_request:
    branches: [master]
  push:
    branches: [master]

jobs:
  check:
    name: build / test / clippy
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Install system dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
            libx11-dev libxext-dev libwayland-dev libxkbcommon-dev libgl1-mesa-dev

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy

      - name: Cache cargo artifacts
        uses: Swatinem/rust-cache@v2

      - name: Build
        run: cargo build --workspace --locked

      - name: Test
        run: cargo test --workspace --locked

      - name: Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 2: Validate YAML syntax**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml')); print('ok')"`
Expected: `ok`

- [ ] **Step 3: Confirm the commands work locally (sanity check)**

Run: `cargo build --workspace --locked && cargo clippy --workspace --all-targets -- -D warnings`
Expected: builds clean; clippy exits 0. (If clippy reports pre-existing warnings, note them — they will fail CI. Fix or report before proceeding.)

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add build/test/clippy workflow for PRs to master"
```

---

### Task 2: Add release-please config and manifest

**Files:**
- Create: `release-please-config.json`
- Create: `.release-please-manifest.json`

- [ ] **Step 1: Write the release-please config**

Create `release-please-config.json`:

```json
{
  "$schema": "https://raw.githubusercontent.com/googleapis/release-please/main/schemas/config.json",
  "release-type": "rust",
  "separate-pull-requests": false,
  "plugins": [
    "cargo-workspace",
    {
      "type": "linked-versions",
      "group-name": "allcast",
      "components": ["allcast", "allcast-sender", "allcast-receiver"]
    }
  ],
  "packages": {
    "allcast": { "component": "allcast" },
    "sender": { "component": "allcast-sender" },
    "receiver": { "component": "allcast-receiver" }
  }
}
```

- [ ] **Step 2: Write the manifest seeded at current versions**

Create `.release-please-manifest.json`:

```json
{
  "allcast": "0.1.0",
  "sender": "0.1.0",
  "receiver": "0.1.0"
}
```

- [ ] **Step 3: Validate both JSON files parse**

Run: `python3 -c "import json; json.load(open('release-please-config.json')); json.load(open('.release-please-manifest.json')); print('ok')"`
Expected: `ok`

- [ ] **Step 4: Verify package keys match real directories**

Run: `for d in allcast sender receiver; do test -f "$d/Cargo.toml" && echo "$d ok"; done`
Expected: `allcast ok`, `sender ok`, `receiver ok` (the `packages` keys are directory paths, not crate names).

- [ ] **Step 5: Commit**

```bash
git add release-please-config.json .release-please-manifest.json
git commit -m "ci: add release-please config for linked workspace versioning"
```

---

### Task 3: Add release-please workflow

**Files:**
- Create: `.github/workflows/release-please.yml`

- [ ] **Step 1: Write the release-please workflow**

Create `.github/workflows/release-please.yml`:

```yaml
name: release-please

on:
  push:
    branches: [master]

permissions:
  contents: write
  pull-requests: write

jobs:
  release-please:
    runs-on: ubuntu-latest
    steps:
      - name: Run release-please
        uses: googleapis/release-please-action@v4
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
          config-file: release-please-config.json
          manifest-file: .release-please-manifest.json
```

- [ ] **Step 2: Validate YAML syntax**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release-please.yml')); print('ok')"`
Expected: `ok`

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release-please.yml
git commit -m "ci: add release-please workflow on master"
```

---

### Task 4: Add CONTRIBUTING.md

**Files:**
- Create: `CONTRIBUTING.md`

- [ ] **Step 1: Write CONTRIBUTING.md**

Create `CONTRIBUTING.md`:

```markdown
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
3. Once merged, release-please maintains a "release" PR; merging it tags the release.

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
```

- [ ] **Step 2: Verify it renders as valid markdown (no broken fences)**

Run: `grep -c '^```' CONTRIBUTING.md`
Expected: an even number (all code fences closed).

- [ ] **Step 3: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs: add CONTRIBUTING with branch flow and commit convention"
```

---

### Task 5: Push the feature branch and open the PR into `master`

**Files:** none (git/GitHub state)

- [ ] **Step 1: Confirm all work is committed on the working branch**

Run: `git status --porcelain`
Expected: empty output.

- [ ] **Step 2: Push the feature branch and open a PR into `master`**

```bash
git push -u origin HEAD
gh pr create --base master --title "ci: add CI + release-please workflows and contributing docs" \
  --body "Adds CI (build/test/clippy), release-please config + workflow, and CONTRIBUTING. Spec: docs/superpowers/specs/2026-06-03-ci-release-workflows-design.md"
```
Expected: PR URL printed; CI workflow begins running against the PR.

- [ ] **Step 3: Verify CI triggered on the PR**

Run: `gh pr checks`
Expected: the `build / test / clippy` check is listed (pending or passing).

---

### Task 6: Apply branch protection via `gh api`

**Files:** none (GitHub repo settings). Requires repo **admin**.

- [ ] **Step 1: Confirm admin access**

Run: `gh api repos/aubrey-silvey/allcast --jq '.permissions.admin'`
Expected: `true`. (If `false`, stop — hand these commands to a repo admin.)

- [ ] **Step 2: Protect `master` (require PR + passing CI check)**

```bash
gh api -X PUT repos/aubrey-silvey/allcast/branches/master/protection \
  -H "Accept: application/vnd.github+json" --input - <<'JSON'
{
  "required_status_checks": { "strict": true, "contexts": ["build / test / clippy"] },
  "enforce_admins": false,
  "required_pull_request_reviews": { "required_approving_review_count": 1 },
  "restrictions": null
}
JSON
```
Expected: JSON response describing the protection rule (HTTP 200).

> Notes:
> - The status check context is the workflow **job name** as GitHub reports it
>   (`build / test / clippy`). Confirm with `gh pr checks` and use that exact value.
> - For a solo maintainer, set `required_approving_review_count` to `0` — still requires a PR +
>   green CI, but allows self-merge without a second reviewer.

- [ ] **Step 3: Verify the rule exists**

Run: `gh api repos/aubrey-silvey/allcast/branches/master/protection --jq '.required_pull_request_reviews != null'`
Expected: `true`.

---

### Task 7: Merge the PR and verify release-please

**Files:** none (git/GitHub state)

- [ ] **Step 1: Merge the PR into `master` once CI is green**

Run: `gh pr merge --squash --delete-branch`
Expected: PR merged into `master`; the release-please workflow runs on the push.

> This bootstrap PR is `ci:`/`docs:` only, so release-please will **not** open a Release PR yet —
> that is expected. The first Release PR appears once the first `feat:`/`fix:` commit lands on
> `master`. To bootstrap a release immediately instead, squash-merge with a `feat:` title.

- [ ] **Step 2: Verify release-please ran**

Run: `gh run list --workflow=release-please.yml --limit 1`
Expected: a completed run against `master`. (No open Release PR yet if history is `ci:`/`docs:` only.)

- [ ] **Step 3: (Later) Confirm the first real feature opens a Release PR**

After a `feat:`/`fix:` commit lands on `master`, run: `gh pr list --state open --search "release"`
Expected: a "chore: release ..." PR with `CHANGELOG.md` and bumped crate versions. Merging it
creates the `vX.Y.Z` tag and GitHub Release (`gh release list`).

---

## Self-Review

**Spec coverage:**
- PR-into-master gating → Task 1 (CI) + Task 6 (master protection). ✓
- master tagged releases → Task 3 + Task 7. ✓
- Release notes from commits → Task 2 + Task 3 (release-please). ✓
- Single shared version → Task 2 (`linked-versions` + `cargo-workspace`). ✓
- Conventional Commits documented, not enforced → Task 4. ✓
- Branch protection via `gh api` → Task 6. ✓
- System GStreamer/GUI deps in CI → Task 1 Step 1. ✓
- YAGNI (no binaries/crates.io) → not present. ✓

**Placeholder scan:** Concrete content in every step. `<number>` / `<release-pr-number>` are runtime values from prior command output, not plan placeholders.

**Consistency:** Status-check context name (`build / test / clippy`) is defined by the job `name:` in Task 1 and reused in Task 6 with a note to reconcile against `gh pr checks` actual output. Package keys are directory paths (`allcast`/`sender`/`receiver`) consistently in Task 2.
