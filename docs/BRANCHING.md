# Branching and release model

Enforced in CI by [`.github/workflows/branch-flow.yml`](../.github/workflows/branch-flow.yml).
This document is the human-readable contract; the workflow is the mechanical
enforcement of it.

## Branches

| Branch | Lifetime | Contains | Merges into |
| --- | --- | --- | --- |
| `main` | permanent | Released, tagged code only. Never has unreleased commits. | — |
| `develop` | permanent | Everything accepted so far that is not yet released. Always releasable. | `main` via the release PR |
| `feature/<name>` | short | One change. Branched from `develop`. | `develop` |
| `fix/<name>` | short | One bug fix. Branched from `develop`. | `develop` |
| `hotfix/<name>` | short | A production fix. Branched from `main`, not `develop`. | `main`, then back into `develop` |

Allowed topic prefixes: `feature/`, `fix/`, `chore/`, `refactor/`, `perf/`,
`docs/`, `test/`, `ci/`, `build/`, `dependabot/`.

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
2. Push `develop`. `release-please.yml` evaluates every commit since the last
   `v*` tag and opens **"chore(release): vX.Y.Z"** into `main`, having bumped:
   - `CHANGELOG.md`
   - `package.json`
   - `src-tauri/Cargo.toml`
   - `src-tauri/tauri.conf.json`
3. Review that PR. It is the changelog users will read, so this is where a
   mislabelled `feat` gets caught — a `feat` that ships as a `patch` under-
   reports a breaking change.
4. Merge it. release-please tags the merge commit `vX.Y.Z`.
5. That tag triggers `release.yml`: version verification, signed builds on
   Linux/macOS/Windows, and a draft GitHub Release with the CHANGELOG section
   as its notes.
6. **Review the draft and publish it.** It stays a draft on purpose.

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

Configure under **Settings → Branches → Branch protection rules**. The
`branch-flow.yml` checks are advisory annotations; protection rules are what
actually block a merge.

### `main`

| Setting | Value |
| --- | --- |
| Require a pull request before merging | on |
| Require approvals | 1 (2 for anything under `src-tauri/src/crypto.rs`) |
| Require status checks | `branch flow / validate branch flow`, `conventional commit title`, `CI / frontend`, `CI / rust`, `CI / build (ubuntu-latest)`, `CI / build (macos-latest)`, `CI / build (windows-latest)` |
| Require branches to be up to date | on |
| Restrict who can push | release-please + humans; never Dependabot |

Note that "Require status checks" cannot express *which branches* may target
`main`. That is the one thing `branch-flow.yml` covers, so both are needed:
protection blocks a bad merge, branch-flow explains why.

### `develop`

| Setting | Value |
| --- | --- |
| Require a pull request before merging | on |
| Require approvals | 1 |
| Require status checks | the same CI set |

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