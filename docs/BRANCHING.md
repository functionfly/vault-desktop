# Branching and release model

Enforced in CI by [`.github/workflows/branch-flow.yml`](../.github/workflows/branch-flow.yml).
This document is the human-readable contract; the workflow is the mechanical
enforcement of it.

## Branches

| Branch | Lifetime | Contains | Merges into |
| --- | --- | --- | --- |
| `main` | permanent | Released, tagged code only. Never has unreleased commits. | — |
| `develop` | permanent | Everything accepted so far that is not yet released. Always releasable. | `main` via a promotion PR |
| `feature/<name>` | short | One change. Branched from `develop`. | `develop` |
| `fix/<name>` | short | One bug fix. Branched from `develop`. | `develop` |
| `hotfix/<name>` | short | A production fix. Branched from `main`, not `develop`. | `main`, then back into `develop` |
| `promote/<name>` | short | A promotion of `develop` into `main`. Branched from `develop`. | `main` |

Allowed topic prefixes: `feature/`, `fix/`, `chore/`, `refactor/`, `perf/`,
`docs/`, `test/`, `ci/`, `build/`, `dependabot/`, `promote/`.

`promote/` is the one prefix that targets `main` without being a release.
It exists because **release-please scans `target-branch`'s commit history**,
and `target-branch` is `main`. A commit therefore only becomes releasable
once it has been promoted into `main`. Without a promotion step the release
PR would never contain the work it is supposed to release.

## Why there is a `develop` branch at all

`main` is the contract the updater and the download page depend on: every tag
on it corresponds to a shipped binary. A 20-person team does not need that
friction on every commit, so ordinary work lands on `develop` and only a
versioned, changelogged, signed artefact is allowed to reach `main`.

The practical effect is that `main` is always bisectable to a real release and
never contains a half-finished commit.

## The daily loop

```bash
git checkout develop
git pull
git switch -c feature/my-change
# ... work ...
git push -u origin feature/my-change
```

Open a PR **into `develop`**. The title must be a Conventional Commit —
[the title is validated](../.github/PULL_REQUEST_TEMPLATE.md) and
`branch-flow.yml` rejects it otherwise, because release-please derives the
SemVer bump from those titles.

| Type | Bump | Example |
| --- | --- | --- |
| `fix`, `perf`, `refactor`, `revert` | patch | `fix(updater): retry once on 503` |
| `feat` | minor | `feat: bulk delete across namespaces` |
| `feat!:` or a `BREAKING CHANGE:` body | major | `feat!: drop PBKDF2 v1 read support` |
| `docs`, `chore`, `ci`, `build`, `test` | none | `ci: cache bun deps` |

`docs`/`chore`/`ci` still appear in the CHANGELOG but do not move the version,
so a docs-only PR produces no release. That is intended.

## Cutting a release

Nothing is tagged by hand.

1. Merge your topic PR into `develop`.
2. Promote `develop` into `main` with a `promote/*` PR. This is the step
   that makes the work visible to release-please: with `target-branch:
   main`, release-please scans **main's** commit history, so a commit that
   is still only on `develop` is not in any release.
3. release-please evaluates every commit on `main` since the last `v*`
   tag. If any of them warrant a SemVer bump it opens **"chore(release):
   vX.Y.Z"** into `main`, having bumped:
   - `CHANGELOG.md`
   - `package.json` (`$.version`)
   - `src-tauri/Cargo.toml` (`$.package.version`)
   - `src-tauri/tauri.conf.json` (`$.version`)
   All four rules live in `release-please-config.json`.
4. Review that PR. It is the changelog users will read, so this is where a
   mislabelled `feat` gets caught — a `feat` that ships as a `patch` under-
   reports a breaking change.
5. Merge it. release-please creates the `vX.Y.Z` tag and a **draft**
   GitHub Release. The tag is what triggers `release.yml`.
6. `release.yml` verifies the tag against every version source, builds +
   signs installers on Linux/macOS/Windows, and uploads them into the
   draft release.
7. **Review the draft and publish it.** It stays a draft on purpose.

### One-time setup still outstanding

The automated path does not run today. Two blockers, both environmental:

| Blocker | Effect | Fix |
| --- | --- | --- |
| Enterprise policy forbids `GITHUB_TOKEN` from creating PRs | release-please cannot open its release PR | Add a `RELEASE_PLEASE_TOKEN` secret (`contents: write`, `pull-requests: write`) |
| release-please v4 reads its config from the **target** branch | `release-please-config.json` and `.release-please-manifest.json` must exist on `main`, not just `develop`, or the action fails with *"Missing required manifest config"* | Merge both files to `main` once — they are repo config, not unreleased code |

Both are repository/secret operations rather than code changes, which is why
they are not automated. Until then use the manual procedure below.

> **Bootstrapping `main` safely.** `branch-flow.yml` permits a
> `release-please--*` branch, a `promote/*` branch, or a `hotfix/*` branch
> to target `main`, and release-please cannot open its PR until the config
> is already on `main`. The way out is a `hotfix/*` branch cut from `main`
> that adds the two config files, merged through the normal PR flow.
> `hotfix/*` → `main` is explicitly permitted by the branch-flow workflow.
>
> Note that a promotion PR and the release PR are two different PRs: the
> promotion carries the *code*, the release PR carries the *version bump
> and the changelog*.

> **Why `skip-github-release` was removed.** The config used to set
> `"skip-github-release": true`. That option makes release-please skip
> creating the release entirely — including the git tag — which means
> `release.yml` (triggered by the `v*` tag push) never fires. The config
> now sets `"draft": true` and `"force-tag-creation": true` instead:
> release-please creates the tag explicitly and leaves the GitHub Release
> as a draft for human review.

## Hotfixes

When production is broken and `develop` does not yet contain the fix:

```bash
git switch -c hotfix/urgent main
# ... fix, with a regression test ...
git push -u origin hotfix/urgent
```

Open a PR **into `main`**. `branch-flow.yml` permits this specifically so a
production regression does not have to wait for `develop`. After merging, back-merge
into `develop` or the fix is lost at the next release:

```bash
git checkout develop
git merge --no-ff hotfix/urgent
git push origin develop
```

The workflow posts a reminder into the job summary when a hotfix is merged.

## Branch protection

> **These rules are NOT currently applied.** The repository is private and
> the owning organisation is on GitHub's free plan, and the branch
> protection API returns `403 Forbidden` for it:
>
> ```
> $ gh api repos/functionfly/vault-desktop/branches/main/protection
> HTTP 403: Resource not accessible by integration
> ```
>
> Rulesets are unavailable for the same reason. The table below is the
> **intent** — apply it verbatim when the plan allows it. Until then, the
> only enforcement is [`branch-flow.yml`](../.github/workflows/branch-flow.yml),
> which checks the head→base pairing and the Conventional Commit title on
> every pull request. It does **not** prevent a direct push to `main` or
> `develop`.
>
> Re-applying the rules (or changing them) needs a token with
> `administration: write` **and** an org plan that permits branch
> protection on a private repository.

The check names below must match the job names in the workflow files
character-for-character. A required check that never matches a real job is
accepted silently by the API and simply never blocks anything, so a typo
weakens the gate without any error. Verify after any rename:

```bash
gh api repos/functionfly/vault-desktop/branches/main/protection \
  --jq '.required_status_checks.contexts'
```

### `main`

| Setting | Value |
| --- | --- |
| Require a pull request before merging | on |
| Require approvals | 1 |
| Require code owner reviews | on |
| Require last push approval | on |
| Dismiss stale reviews | on |
| Require branches to be up to date | on (`strict`) |
| Require linear history | on |
| Force pushes / deletions | blocked |
| Required status checks | `CI / frontend (tsc + vitest)`, `CI / rust (fmt + clippy + test)`, `CI / build (ubuntu-latest)`, `CI / build (macos-latest)`, `CI / build (windows-latest)`, `CI / release scripts`, `branch flow / validate branch flow`, `conventional commit title` |

### `develop`

Identical required checks, with these differences:

| Setting | Value | Why |
| --- | --- | --- |
| Require branches to be up to date | off (`strict: false`) | Topic branches must be able to merge without a merge commit. |
| Require linear history | off | Allows the hotfix back-merge. |
| Require code owner reviews | off | Faster day-to-day iteration. |
| Require last push approval | off | Same. |

## Releasing without release-please (current workaround)

The owner org is under an **enterprise policy that forbids the Actions token
from creating or approving pull requests**:

```
The enterprise does not allow GitHub Actions to create or approve pull requests
```

`release-please` needs exactly that, so it cannot open its release PR here.
Until a PAT is provisioned, cut a release by hand:

```bash
git switch main
git pull

# 1. Bump all four version sources to the same value.
#    package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json
#    (.release-please-manifest.json records what release-please last tagged.)

# 2. Add the CHANGELOG section.

# 3. Commit, merge via PR, then tag:
git tag -a v0.1.1 -m "v0.1.1"
git push origin v0.1.1
```

To restore the automated path, add a `RELEASE_PLEASE_TOKEN` secret
(contents: write, pull-requests: write). The workflow already prefers it and
falls back to `GITHUB_TOKEN`.

## Commit signing

`git config commit.gpgsign true` locally. The CHANGELOG is not generated from
commit signatures, but signed history is what makes `git verify-tag` on the
release merge commit meaningful to an auditor.

## Version files

The version lives in three places and they must always agree:

| File | Purpose |
| --- | --- |
| `package.json` | npm package identity |
| `src-tauri/Cargo.toml` | crate identity |
| `src-tauri/tauri.conf.json` | the version Tauri stamps into the binary and the updater manifest |

`release.yml` fails closed when the `v*` tag disagrees with any of them, so a
partial bump cannot ship a mislabelled installer. **Do not edit the version in
any of these files by hand** — that is what the release PR is for.