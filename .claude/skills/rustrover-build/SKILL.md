---
name: rustrover-build
description: Build or run the Prism desktop app through the RustRover MCP connection (run configurations "Prism Build" / "Prism Run"), never with `cargo build` / `cargo run` in the shell. Use whenever a change needs the app compiled, launched or checked in the real app ("빌드해", "앱 실행", "build the app", "run Prism", "확인해 봐"). A PreToolUse hook in .claude/settings.local.json blocks shell `cargo build|run`.
---

# Building and running Prism through RustRover

The maintainer requires every app build and launch to go through RustRover over MCP, so it
uses their shared target directory and environment (`DEVELOPER_DIR=/Library/Developer/CommandLineTools`)
and shows up in the IDE. A hook in `.claude/settings.local.json` denies shell `cargo build`,
`cargo b`, `cargo run` and `cargo r`.

## Steps

1. If the `mcp__rustrover__*` tools are deferred, load them first:
   `ToolSearch` with `select:mcp__rustrover__get_run_configurations,mcp__rustrover__execute_run_configuration,mcp__rustrover__build_project`.
2. List the configurations once per session:
   `mcp__rustrover__get_run_configurations` with
   `projectPath: /Volumes/NKStudioHD/1.DevProjects/RustProject/photocraft`.
   Prefer one named "Dev Prism Run" if it exists; otherwise:
   - **Prism Build**: `build --locked -p prism`
   - **Prism Run**: `run --locked -p prism`
3. Build: `mcp__rustrover__execute_run_configuration` with
   `configurationName: "Prism Build"`, `waitForExit: true`, `timeout: 1200000` and the same
   `projectPath`. Read `exitCode` and the output snapshot; on errors open `fullOutputPath`
   (Read it, or `grep -E "^(error|warning)|-->"` it) for the full log.
4. Run the app: `configurationName: "Prism Run"` with `waitForExit: false` (the app keeps
   running). To drive it over the control channel, pass the extra arguments for this launch only
   with `programArguments` (for example `-- --control 7890`; the configuration supports dynamic
   launch overrides).
5. Report the result to the user in Korean, including a failed build's first errors.

## What is still fine in the shell

- `cargo test`, `cargo clippy`, `cargo fmt`, `cargo xtask …`, `cargo metadata` (not blocked).
  Use the default target dir (no `CARGO_TARGET_DIR=target/agent-*`) and
  `DEVELOPER_DIR=/Library/Developer/CommandLineTools`, as the run configurations do.
- If RustRover isn't reachable (MCP tools missing or erroring), say so and ask the user to open
  the project in RustRover; don't fall back to shell `cargo build`.
