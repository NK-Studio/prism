---
name: release
description: Walk the maintainer through a local Prism release, one step at a time, with the conveyor belt in `cargo xtask release` (packaging/release/pipeline.json, progress in log/release/state.json). Use when they ask what to do next for a release or deploy ("배포 다음 뭐해?", "릴리즈 시작", "what's next for the release"), say a release step is finished ("완료했어", "done", "빌드 끝났어"), want to skip a step, go back, or start a new version.
---

# Prism release conveyor belt

The release is a fixed sequence of steps in `packaging/release/pipeline.json`. Progress lives in
`log/release/state.json` (local, never committed). **Only `cargo xtask release` reads or writes the
state.** Never edit state.json by hand and never mark a step done without running the command:
`done` runs the step's checks first, and that is the point of the belt.

Run cargo as `cargo`, or `~/.cargo/bin/cargo` if it isn't on PATH. Unset a stale `DEVELOPER_DIR`
if linking fails with "missing DEVELOPER_DIR path". Reply in the user's language (Korean for this
maintainer); command names, file paths and versions stay as they are.

## What to do for each request

**"What's next?" / "where are we?"**
1. `cargo xtask release status --json`.
2. If it errors with "no release in progress", ask which version to release (suggest the next
   patch/minor after `cargo xtask version`), then `cargo xtask release start X.Y.Z`.
3. Explain the current step plainly: what to do, where (RustRover run configuration or terminal),
   what "done" means. Show each command in its own ```bash block. Mention when a step is optional.
4. Offer to do the parts you can do yourself (running a script, a commit, a tag) — but ask before
   anything outward-facing (push, tag push, GitHub Release, publishing the feed).

**"Done!" / "완료했어"**
1. `cargo xtask release done` (add `--note "..."` with anything worth remembering, e.g. a skipped
   detail or a build number).
2. If a check fails, say exactly which one and what it found, and how to fix it; the step stays
   current. Don't retry in a loop and don't work around the check.
3. If it passes, the command prints the next step: explain that one (as above).

**"Skip this"** — only optional steps can be skipped: `cargo xtask release skip --note "why"`.
For a required step, explain why it matters instead.

**"Go back" / something went wrong in an earlier step** — `cargo xtask release back` reopens the
previous step (repeat to go further back).

**"Is it done?" without wanting to move on** — `cargo xtask release check`.

**New version while one is unfinished** — confirm with the user, then
`cargo xtask release start X.Y.Z --force`.

## Background the steps rely on

- Builds are local (GitHub Actions is off for billing reasons). macOS is Apple silicon only:
  RustRover "Prism Package (macOS)" with ▶ (the hammer only builds the binary) or
  `packaging/macos/package.sh --arch aarch64`. Windows needs a Windows PC (`packaging/windows/package.ps1`).
- First-time installers (.dmg) are shared only on the internal Confluence page, never publicly.
  The `confluence` step has no automatic check: mark it done when the maintainer says it's attached.
- Without Developer ID secrets the app is ad-hoc signed: in-app updates work, but a .dmg
  downloaded from the web is blocked by Gatekeeper. Once users have a Developer ID–signed build,
  later updates must use the same Team ID or Prism refuses them.
- Publishing the update: RustRover "Prism Publish Update (dry run)" then "Prism Publish Update",
  or `packaging/publish-update.sh`. It signs with `~/prism-update.key`, force-pushes a single
  commit to NK-Studio/prism-updates (pushing with the `NK-Studio` gh account) and waits for
  https://prism-updates.vercel.app/stable/latest.json to report the version. The site keeps the
  newest three versions.
- The first release that contains in-app updates can't reach anyone through the updater: skip
  `check-update` for it and tell users to install it by hand.
- Details: docs/releasing.md › In-app updates.

## Changing the belt

Steps, instructions and checks are data in `packaging/release/pipeline.json` (check kinds:
`cargoVersion`, `gitClean`, `files`, `command`, `urlJson`; `{version}` is substituted). Edit it
when the process changes; `cargo test -p xtask release` validates it.
