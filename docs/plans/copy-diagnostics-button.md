# Copy Diagnostics in the title bar: implementation plan

> **Status:** proposed, with [the spec](../specs/copy-diagnostics-button.md) · **Written:** 2026-10-15 · **Change:** major (first version)

One pull request (1–2 h), `cargo xtask ci` green; task id `D1` in the commit message. Requirement
numbers are the spec's.

| Part | What |
|---|---|
| Pill | In `crates/ui-egui/src/chrome.rs` `title_bar`, in the right-to-left group after the avatar: a rounded pill with the "copy" icon and "Copy Diagnostics" through `tl!`, hover colour from `Tokens`, tooltip (S1, S3, S4) |
| Command | The click runs `ui.copyDiagnostics` through `app.run` (S2) |
| Test | A UI test that the pill is there and that clicking it runs the command; screenshots light and dark, wide and narrow |
| Docs | `docs/ui-parity.md` row; ROADMAP progress log |

**Done when:** the pill shows in light and dark, copies the same text as Help › Copy Diagnostics,
and the search box gives way to it on a narrow window.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version |
