# Holm

- Run `cargo fmt`, then `scripts/check`, before you finish. Leave no warning.
- Profile with the `profile` skill.
- See the app with the `look` skill.
- Delete each local branch whose PR `gh pr view <branch> --json state` shows as `MERGED`: switch to `master`, pull, run `git branch -D <branch>`. A rebase merge changes the hashes, so `-d` refuses it.
