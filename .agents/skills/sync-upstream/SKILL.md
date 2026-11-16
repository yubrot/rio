---
name: sync-upstream
description: Update this Rio fork from raphamorim/rio by fast-forwarding main to upstream/main and rebasing yubrot-fork onto main. Use when asked to sync with upstream or follow the latest upstream changes.
---

# Sync upstream

Run from the repository root. The intended sequence is: check out `main`,
update it from upstream, then rebase `yubrot-fork` onto `main`.

1. Check `git status` and `git remote -v`. If there are uncommitted changes or
   an ongoing merge/rebase, stop and explain what needs attention; do not
   automatically stash or discard work.
2. If `upstream` is missing, add it:

   ```sh
   git remote add upstream https://github.com/raphamorim/rio.git
   ```

   If it already exists, confirm it points to `raphamorim/rio` before proceeding.
3. Run these commands in order, stopping on any failure:

   ```sh
   git checkout main
   git fetch upstream
   git merge --ff-only upstream/main
   git checkout yubrot-fork
   git rebase main
   ```

   If `main` cannot fast-forward, explain the divergence without resetting it.
4. Resolve rebase conflicts while preserving both upstream changes and the
   fork's intent, consulting `AGENTS.md` and `PLAN.md` as needed. Stage resolved
   files explicitly and run `git rebase --continue`. If a conflict requires a
   product decision that cannot be inferred, ask the user and leave the rebase
   in progress. Do not drop fork commits just to make the rebase succeed.
5. Verify that the current branch is `yubrot-fork`, the working tree is clean,
   and `git merge-base --is-ancestor main yubrot-fork` succeeds. Run checks
   appropriate to any conflict resolutions, following `AGENTS.md`.

Report the resulting upstream commit, whether conflicts occurred, and any
checks performed. Do not push unless the user requests it; pushing the rebased
fork requires `--force-with-lease`.
