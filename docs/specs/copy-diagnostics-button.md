# Copy Diagnostics in the title bar

> **Status:** proposed · **Written:** 2026-10-15 · **Author:** Martin Ball, with Claude · **Change:** major (first version) · **Target:** MailCraft's own (Outlook has no such button)

The words in this document have the meanings in [`CONTEXT.md`](../../CONTEXT.md). From the R3.2
decision session with Martin Ball on 2026-10-15. The work is in
[`docs/plans/copy-diagnostics-button.md`](../plans/copy-diagnostics-button.md).

## Why

Testers report problems, and a report is only useful with **diagnostics**. Help › Copy Diagnostics
(`ui.copyDiagnostics`) already copies them, but it is three clicks away and testers don't find it.

## Requirements

1. A labelled pill, "Copy Diagnostics", stands at the right end of the title bar, just left of the
   account's picture, beside the window's own minimise, maximise and close buttons. It is shown
   always while MailCraft is in alpha and beta; whether it stays after 1.0 is decided then.
2. Clicking it runs `ui.copyDiagnostics`, the same command as Help › Copy Diagnostics: nothing new
   is collected, and nothing is sent. The status bar says "Diagnostics copied. Nothing was sent."
3. Its tooltip says what is copied: the version, the platform and the end of the program's own
   record, with no message content, address, password or token.
4. It is drawn in the window's colours (`theme::Tokens`), readable in light and dark, and the
   search box shrinks to make room; on a narrow window the search box goes before the pill does.

## Out of scope

Sending a report anywhere; a switch to hide the pill.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version |
