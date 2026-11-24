# Holm

- Run `cargo fmt`, then `scripts/check`, before you finish. Leave no warning.
- Profile with the `profile` skill.
- See the app with the `look` skill.
- Delete each local branch whose PR `gh pr view <branch> --json state` shows as `MERGED`: switch to `master`, pull, run `git branch -D <branch>`. A rebase merge changes the hashes, so `-d` refuses it.

## Tests

- Put a module's unit tests in its child file `foo/tests.rs`, declared `#[cfg(test)] mod tests;`. Split a big one into `foo/tests/<topic>.rs`.
- Put a test that needs only the public API in `tests/integration/<topic>.rs`, a module of the crate's one integration test binary. Keep a test that needs a private item a unit test, and leave the item private.
- Put a test helper in the nearest place all its users reach: `tests/integration/support.rs` for the integration tests, the crate's `testing` module behind `test-support` for several targets or crates.
- Put files that tests, benches and examples read in `test_data/<kind>/` at the crate root, with paths from `CARGO_MANIFEST_DIR`. Get another crate's data from its `testing` module.
- Time private items in an ignored unit test in a `speed` topic, and the public API in `benches/<topic>.rs`.
- Turn a fuzz crash into a normal test.
