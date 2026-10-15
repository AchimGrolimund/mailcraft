# Roadmap reset: implementation plan

> **Status:** accepted 2026-10-14, with [the spec](../specs/roadmap-reset.md); tracked in [#2](https://github.com/billsecond/mailcraft/issues/2) · **Written:** 2026-10-14 · **Change:** minor (R8.5: working days)

The order of the work to beta. Each step is one pull request or a short series, small enough to
take or refuse on its own. Task ids are `R<step>.<n>` and go in commit messages
(`R3.2: categories reach the server`). Every step ends with `cargo xtask ci` green on all three
CI platforms. **[review]** marks a step a person reviews before it lands (sync, store, sign-in,
secrets). Hours are guesses from the gap list until a step has been done.

## R1. Hygiene (8–12 h) [review]

| Task | What | Done when |
|---|---|---|
| R1.1 | Make `store::tests::the_saver_writes_in_the_background_coalesces_and_flushes` deterministic (wait on the saver's signal, not on the clock) | Passes 200 runs in a row under `stress`-like load on Linux CI |
| R1.2 | The `#![deny(clippy::unwrap_used, …)]` line in `crates/control-key/src/lib.rs` | `cargo xtask ci` green |
| R1.3 | UIDVALIDITY: on a change, keep the folder's pending changes, re-apply each to the message it names where it can be found again (by Message-ID) and tell the owner of the rest (`crates/engine/src/cmd/account.rs`) | A regression test: offline flag + move in a folder whose UIDVALIDITY changes; the flag and move survive, an unfindable one is reported |
| R1.4 | Correct the stale facts: `ROADMAP.md` (releases published, parity 167/310, "one real mailbox"), `docs/gaps.md` gap 6 (the picture cache exists) and gap 18 (four releases), `README.md` (0.2), `docs/roadmap.md` current focus | Each figure matches `cargo xtask parity`, `gh release list` and the code |
| R1.5 | Add `CONTEXT.md`, this plan, the spec and ADRs 0001–0002 | Merged |
| R1.6 | `CODEOWNERS` for the reviewed paths; branch protection on `main` (owner's action) | A PR touching `crates/net` asks for review |

## R2. Remove Exchange Web Services (6–10 h)

| Task | What | Done when |
|---|---|---|
| R2.1 | Delete `crates/net/src/ews.rs`, `ews/`, `oauth::refresh_ews`, `oauth::sign_in_for_ews`, the EWS scope | `grep -ri ews crates apps` finds only `news`-like words |
| R2.2 | Remove `FavoritesSource` and `account.favoritesSource`; a saved mailbox that has it opens, ignores it and says once that Favorites now come through Graph | A test opens a mailbox written with `favoritesSource: "ews"` |
| R2.3 | Remove the choice from Add Account, File › Options and `favorites_sync/choice.rs`; the admin-consent address no longer names EWS | UI tests and screenshots, light and dark |
| R2.4 | `CLAUDE.md`, `AGENTS.md`, `docs/architecture.md`, `docs/gaps.md`: EWS gone from the rules and the clean-room spec list | Reviewed |

## R3. Graph mail for Microsoft accounts (60–100 h) [review]

Detailed in [the R3 spec](../specs/r3-graph-mail.md) and [plan](r3-graph-mail.md) (2026-10-15).

The critical path. Built against the Graph stand-in (`crates/net/src/graph/standin.rs`), held to
the documented API; the sync request/apply split stays (`crates/model/src/sync.rs`), so the
engine does no I/O.

| Task | What | Done when |
|---|---|---|
| R3.1 | Folders: list, create, rename, move, delete through Graph; nested, in Outlook's order | Stand-in flow tests both ways |
| R3.2 | Messages down: delta queries per folder, resumable, in batches, `Retry-After` honoured, progress and Cancel; immutable ids | 200,000 messages from the stand-in, interrupted and resumed, with nothing twice and nothing missing |
| R3.3 | Changes up: read, flag, categories, importance, move, copy, delete, from the existing pending-changes queue; each reaches the server or is refused with a reason | One test per kind; a refused change is put back and said |
| R3.4 | Drafts kept in the server's Drafts, both ways; sending through Graph as today | A draft written here is in Outlook on the web's Drafts (stand-in) and back |
| R3.5 | Undo after the server has it sends the reverse change | Undo of a move and a delete, tested |
| R3.6 | Imported mail goes to a local folder; moving it to a server folder is refused with the reason (Graph can't upload received mail) | Tested |
| R3.7 | The switch for an account synced by IMAP: say what will happen, back up the mailbox file, download afresh, keep local folders, unsent drafts and MailCraft's favourites | A mailbox from 0.2.1 switches and keeps those |
| R3.8 | Microsoft accounts stop using IMAP and SMTP; the XOAUTH2 path stays until R3 is proven on testers, then goes | No IMAP connection for a Microsoft account in the flows |
| R3.9 | The author's own mailbox on the new path for a week before R8 | No data-loss incident |

## R4. Rich-text reply, minimal (25–45 h)

Runs beside R3.

| Task | What | Done when |
|---|---|---|
| R4.1 | A compose body model of paragraphs and runs: bold, italic, underline, bulleted and numbered lists, links, a font size | Model tests; undo inside the message |
| R4.2 | The editor in `ui-egui`: typing, selection, the formatting keys and ribbon buttons | Screenshots light and dark; keyboard tests |
| R4.3 | Reply and forward quote the original's HTML intact below the owner's text (shown through `crates/html`, sent as it came, header-injection rules kept) | A reply to a formatted newsletter keeps its quoted layout in Outlook on the web (stand-in round trip) |
| R4.4 | Paste reduced to the set; HTML out with a plain-text part (`crates/mime`) | Property test: arbitrary pasted HTML never escapes the set |

## R5. Sign-ins remembered on Linux (6–10 h) [review]

| Task | What | Done when |
|---|---|---|
| R5.1 | Secret Service backend in `apps/mailcraft/src/secrets.rs` (the `keyring` crate's Linux feature) | Remembered across a restart on GNOME and KDE |
| R5.2 | No Secret Service running: ask each start as today, and say why once | Tested |

## R6. Bodies on demand (15–25 h) [review]

| Task | What | Done when |
|---|---|---|
| R6.1 | The mailbox keeps headers and the index in memory; bodies and attachments are read from their blob files when shown, searched or sent | 200,000 messages: memory under 1 GB after open |
| R6.2 | Full-text search reads bodies through a bounded cache, or an index of words | Search everywhere at 200,000 under 1 s |
| R6.3 | The first save after a large download runs in the background with progress | The window never waits for it |
| R6.4 | Re-measure (the 2026-10-14 benchmark, kept as an example) and write the figures into `ROADMAP.md` | B8 measured |

## R7. Notifications and diagnostics (12–20 h)

| Task | What | Done when |
|---|---|---|
| R7.1 | System notifications for reminders and new mail while MailCraft runs (Windows, macOS, Linux); off-switch in File › Options | Seen on each platform |
| R7.2 | `ui.copyDiagnostics`: version, platform, the redacted Send/Receive log; no content, addresses, passwords or tokens | A test feeds a log with all of those and finds none in the copy |

## R8. The beta run (2–4 weeks of calendar time)

| Task | What | Done when |
|---|---|---|
| R8.1 | Signed builds and publisher verification (owner's action) | B10 |
| R8.2 | A tester issue template (with Copy diagnostics) and labels `data-loss`, `crash`, `out-of-step` | In the repository |
| R8.3 | Fold `docs/roadmap.md` and `docs/target-app-parity.md` into `ROADMAP.md` and `docs/gaps.md`; the beta checklist in `ROADMAP.md` | Two roadmap documents |
| R8.4 | Recruit at least 5 testers across the three platforms (start during R3) | B1, B3 |
| R8.5 | 14 working days; every `data-loss` or `crash` issue fixed with a regression test, and the 14 working days restart for that tester | B2, B4–B9 pass |

## Order and overlap

```
R1 ─ R2 ─ R3 ─────────────────── R8
          R4 ──────────┘
          R5 ─ R6 ─ R7 ┘
```

Total before the beta run: about 130–220 hours of focused work.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-14 | major | First version |
| 2026-10-15 | minor | R3 detailed in its own spec and plan; R3.6: imported mail can't be uploaded through Graph |
| 2026-10-15 | minor | R8.5: 14 working days (spec B2) |
