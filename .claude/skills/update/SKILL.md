---
name: update
description: Updates Holm's Rust toolchain, its Cargo.lock, the action pins of its workflows and the tool versions in scripts/check, and opens one PR for it all. Use when the weekly toolchain check fails, or the user asks to update dependencies.
---

# Update

Run this at each Rust release. The toolchain job of `advisories.yml` fails when `rust-toolchain.toml` is older than the newest stable Rust.

## Not in it

The user moves each of these in its own PR:

- a new major version of a crate in `Cargo.toml` or of an action;
- GPUI: the `gpui-kit = "=0.9.0"` pin in `Cargo.toml` and the changed copies in `vendor/`;
- `ropey`'s beta pin.

## Steps

Work on a new branch from `master`, like `update/rust-1.100.0`. Make one commit per step.

1. **Toolchain.** Read the newest stable Rust:

   ```sh
   curl -fsSL https://static.rust-lang.org/dist/channel-rust-stable.toml \
     | sed -n '/^\[pkg\.rust\]$/,/^version = /s/^version = "\([0-9.]*\) .*/\1/p'
   ```

   Set `channel` in `rust-toolchain.toml` to it. Commit: `build: move to rust 1.100.0`.
2. **Lints.** Run `scripts/check check` and fix each new lint and warning. Commit: `refactor: fix the lints of rust 1.100`.
3. **Crates.** Run `cargo update`, then `scripts/check`. Only `Cargo.lock` changes. Hold back a `gpui-*` crate that breaks the build with `cargo update -p <crate> --precise <old>`. Commit: `build(deps): update Cargo.lock`.
4. **Pins and tools.**
   - `actions/checkout` and `Swatinem/rust-cache` are pinned by commit hash in `.github/workflows/*.yml`. Read the version behind each hash. Take the full tag, like `v7.0.1`, not `v7`:

     ```sh
     gh api --paginate repos/<owner>/<repo>/tags --jq '.[] | select(.commit.sha == "<hash>") | .name'
     ```

     Read the newest release in that major version, and its commit:

     ```sh
     gh api --paginate repos/<owner>/<repo>/releases --jq '.[] | select(.draft or .prerelease | not) | .tag_name'
     gh api repos/<owner>/<repo>/commits/<tag> --jq .sha
     ```

     Replace every use of the old hash.
   - Set `cargo_deny_version` and `cargo_about_version` in `scripts/check` to the newest versions, from `cargo info <tool>`, across major versions too. Read each changelog for changed flags or config. Install each with the command `scripts/check` prints, then run `scripts/check deny` and `scripts/check licenses`.

   Commit: `ci: refresh action pins and tool versions`.
5. **PR.** Push the branch and open the PR with `gh pr create`. List the old and new version of the toolchain, each action and each tool. List each upgrade left out. The user merges.

## Rules

- Ask the user before you change `about.toml` or `deny.toml`. A new licence or crate source is their choice.
- Ask the user before a fix bigger than a few lines.
- Skip steps 1 and 2 when `rust-toolchain.toml` is already the newest stable.
