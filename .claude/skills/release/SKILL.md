---
name: release
description: Bumps Holm's version, drafts its CHANGELOG.md section from the commits since the last release, and opens the bump PR. After the merge, it tells the user to start release.yml. Use when the user wants a new release of Holm.
---

# Release

`release.yml` publishes the version in `Cargo.toml`, with its `CHANGELOG.md` section as the notes. This skill puts both on `master` through one PR. The first release, `0.1.0`, needs no PR: it is the button alone.

## Steps

Work on a new branch from an up-to-date `master`, like `release/0.2.0`.

1. **Commits.** Read the commits since the last release:

   ```sh
   git fetch --tags
   last=$(git describe --tags --abbrev=0 --match 'v*' master)
   git log --reverse --format='%h %s%n%b' "$last"..master
   ```

2. **Version.** Bump the leftmost number the commits allow to move:
   - a break moves the first number: a commit with `!` or a `BREAKING CHANGE:` footer, or anything a user relied on that now works differently;
   - a feature a user can see moves the middle number;
   - fixes alone move the last number.

   At `0.x`, a break moves the middle number. Leave `0` to `1` to the user. Ask the user to confirm the version, with the commits that decided it.
3. **Bump.** Set `version` in `[workspace.package]` of `Cargo.toml`. Run `cargo check` so `Cargo.lock` follows.
4. **Notes.** Add a `## X.Y.Z` section, with no date, at the top of `CHANGELOG.md`. Write it for people who download Holm. Use these parts, in this order, each only with entries:
   1. one summary sentence, for a big release;
   2. `### Before you update`: what the user must do, with exact steps;
   3. `### New`: each feature, with the key or menu that reaches it;
   4. `### Changed`: including anything that touches notes or settings;
   5. `### Fixed`: each bug as the user saw it.

   Write one sentence per bullet, plus one for a step the user must take. Merge many tiny fixes into one bullet. Leave out internal work (refactors, CI, tests, dependency bumps that fix nothing seen), issue numbers, hashes, thanks and filler. Check each key and menu name against the code.
5. **PR.** Commit `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` together as `chore(release): 0.2.0`. Push the branch and open the PR with `gh pr create`, with the section as its body. The user edits the notes on the branch and merges.
6. **Publish.** After the merge, tell the user to press "Run workflow" on `release.yml`, or to run `gh workflow run release.yml`. Leave the tag and the release to the workflow, and the notes to `CHANGELOG.md` alone.
