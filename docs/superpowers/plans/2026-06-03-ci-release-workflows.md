# CI & Release Workflows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `dev`-integration / `master`-release branch model with GitHub Actions for CI and release-please-driven tagged releases + changelog.

**Architecture:** Feature branches PR into `dev` (gated by a CI workflow running build/test/clippy with system GStreamer + GUI libs). `dev` is promoted to `master` via PR. release-please watches `master`, maintains a Release PR (CHANGELOG.md + lockstep version bumps across the 3 crates), and on merge creates tag `vX.Y.Z` + a GitHub Release with generated notes. Branch protection on both branches is applied via `gh api`.

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

All work happens on a branch off `master`; the workflow files must reach `master` (via `dev` → PR) before their triggers fire on the intended branches.

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
    branches: [dev]
  push:
    branches: [dev, master]

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
git commit -m "ci: add build/test/clippy workflow for dev and master"
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

### Task 5: Create the `dev` branch and open the integration PR

**Files:** none (git/GitHub state)

- [ ] **Step 1: Confirm all work is committed on the working branch**

Run: `git status --porcelain`
Expected: empty output.

- [ ] **Step 2: Create `dev` from the current `master` and push it**

```bash
git checkout master
git pull origin master
git checkout -b dev
git push -u origin dev
```
Expected: `dev` branch created on origin.

- [ ] **Step 3: Push the feature branch and open a PR into `dev`**

```bash
git checkout -          # back to the workflow feature branch
git push -u origin HEAD
gh pr create --base dev --title "ci: add CI + release-please workflows and contributing docs" \
  --body "Adds CI (build/test/clippy), release-please config + workflow, and CONTRIBUTING. Spec: docs/superpowers/specs/2026-06-03-ci-release-workflows-design.md"
```
Expected: PR URL printed; CI workflow begins running against the PR.

- [ ] **Step 4: Verify CI triggered on the PR**

Run: `gh pr checks`
Expected: the `CI / build / test / clippy` check is listed (pending or passing).

---

### Task 6: Apply branch protection via `gh api`

**Files:** none (GitHub repo settings). Requires admin auth: confirm with `gh auth status`.

- [ ] **Step 1: Confirm admin access**

Run: `gh api repos/aubrey-silvey/allcast --jq '.permissions.admin'`
Expected: `true`. (If `false`, stop — ask the user to run these as an admin.)

- [ ] **Step 2: Protect `dev` (require PR + passing CI check)**

```bash
gh api -X PUT repos/aubrey-silvey/allcast/branches/dev/protection \
  -H "Accept: application/vnd.github+json" \
  -f "required_status_checks[strict]=true" \
  -f "required_status_checks[contexts][]=build / test / clippy" \
  -f "enforce_admins=false" \
  -f "required_pull_request_reviews[required_approving_review_count]=1" \
  -f "restrictions=null"
```
Expected: JSON response describing the protection rule (HTTP 200).

> Note: the status check context name is the workflow **job name** as GitHub reports it
> (`build / test / clippy`). If `gh pr checks` shows a different string, use that exact value here.

- [ ] **Step 3: Protect `master` (require PR; block direct pushes)**

```bash
gh api -X PUT repos/aubrey-silvey/allcast/branches/master/protection \
  -H "Accept: application/vnd.github+json" \
  -f "required_status_checks=null" \
  -f "enforce_admins=false" \
  -f "required_pull_request_reviews[required_approving_review_count]=1" \
  -f "restrictions=null"
```
Expected: JSON response describing the protection rule (HTTP 200).

- [ ] **Step 4: Verify both rules exist**

Run: `gh api repos/aubrey-silvey/allcast/branches/dev/protection --jq '.required_pull_request_reviews != null' && gh api repos/aubrey-silvey/allcast/branches/master/protection --jq '.required_pull_request_reviews != null'`
Expected: `true` then `true`.

---

### Task 7: Merge to dev, promote to master, verify release-please

**Files:** none (git/GitHub state)

- [ ] **Step 1: Merge the PR into `dev` once CI is green**

Run: `gh pr merge --squash --delete-branch`
Expected: PR merged into `dev`.

> Use a `feat:` or `fix:` prefixed merge/squash title so release-please produces a versioned release
> (a `ci:`/`docs:`-only history yields no version bump and no Release PR).

- [ ] **Step 2: Open the promotion PR `dev` → `master`**

```bash
gh pr create --base master --head dev \
  --title "feat: initial CI and release automation" \
  --body "Promote dev to master to bootstrap release-please."
```
Expected: PR URL printed.

- [ ] **Step 3: Merge the promotion PR**

Run: `gh pr merge <number> --merge`
Expected: merged into `master`; the release-please workflow runs on the push.

- [ ] **Step 4: Verify release-please opened a Release PR**

Run: `gh pr list --state open --search "release-please"`
Expected: a "chore(main): release ..." PR exists, with `CHANGELOG.md` and bumped crate versions.

- [ ] **Step 5: (Optional) Cut the first release**

Run: `gh pr merge <release-pr-number> --squash`
Then: `gh release list`
Expected: a `vX.Y.Z` tag and GitHub Release with generated notes appear.

---

## Self-Review

**Spec coverage:**
- PR-into-dev gating → Task 1 (CI) + Task 6 (dev protection). ✓
- master tagged releases → Task 3 + Task 7. ✓
- Release notes from commits → Task 2 + Task 3 (release-please). ✓
- Single shared version → Task 2 (`linked-versions` + `cargo-workspace`). ✓
- Conventional Commits documented, not enforced → Task 4. ✓
- Branch protection via `gh api` → Task 6. ✓
- System GStreamer/GUI deps in CI → Task 1 Step 1. ✓
- YAGNI (no binaries/crates.io) → not present. ✓

**Placeholder scan:** Concrete content in every step. `<number>` / `<release-pr-number>` are runtime values from prior command output, not plan placeholders.

**Consistency:** Status-check context name (`build / test / clippy`) is defined by the job `name:` in Task 1 and reused in Task 6 with a note to reconcile against `gh pr checks` actual output. Package keys are directory paths (`allcast`/`sender`/`receiver`) consistently in Task 2.
