# Roadmap reset: from here to beta

> **Status:** accepted 2026-10-14 ([#1](https://github.com/UCS-Care-Group/mailcraft/pull/1)); tracked in [#2](https://github.com/UCS-Care-Group/mailcraft/issues/2) · **Written:** 2026-10-14 · **Author:** Martin Ball, with Claude · **Change:** minor (B2: working days) · **Target:** Microsoft Outlook (Microsoft 365)

The words in this document have the meanings in [`CONTEXT.md`](../../CONTEXT.md). The order of
the work is in [`docs/plans/roadmap-reset.md`](../plans/roadmap-reset.md); the two decisions that
are hard to undo are [ADR 0001](../adr/0001-microsoft-accounts-first-beta.md) and
[ADR 0002](../adr/0002-graph-mail-for-microsoft-accounts.md).

## Why

A review on 2026-10-14 found MailCraft well built and the plan for it out of step with itself:

- **The code is sound.** `cargo xtask ci` passes (1,573 tests); no panic paths in production
  code; saving is atomic; sync is two-way with guards against a server wiping a folder; every
  reader is property-tested.
- **The roadmap documents contradict each other and the code.** `ROADMAP.md` says no release is
  published (four are), and that nothing has met a real server (its own log says one Microsoft
  365 mailbox is in daily use); `docs/gaps.md` says pictures have no disk cache (they do);
  `docs/target-app-parity.md` is unchanged since 2026-10-11; `docs/roadmap.md`'s current focus
  is work that is done; milestone ids in the log (M3.x, M4.x) don't match the milestone table.
- **The readiness numbers are guesses.** "~25 %" and "~55 %" can't be checked, and nobody but
  the author has used the program.
- **Recent work went sideways.** M4.2–M4.4 read Outlook's Favorites through an undocumented
  Graph beta item and through Exchange Web Services, which Microsoft switches off on
  1 April 2027, while plain-text composing, a mailbox held whole in memory and the gaps in
  two-way sync, all of which block beta, stood still.
- **CI on `main` is red** (a timing-sensitive saver test failed on Linux, 2026-10-13).
- **A data-loss path exists:** when a folder's UIDVALIDITY changes, changes made offline in that
  folder are discarded (`crates/engine/src/cmd/account.rs`, `mb.pending.retain`).

## Goal

Reach **Beta** as `CONTEXT.md` defines it, with nothing else in the way.

## Beta checklist (pass/fail)

Every item must pass. Each is checked by testers' reports and GitHub issues, not by estimate.

| # | Item |
|---|---|
| B1 | At least 5 testers, none of them authors, each on a Microsoft account (personal, or work or school) |
| B2 | Each uses MailCraft on 14 working days (Monday to Friday; about three weeks), on at least 2 devices |
| B3 | Across Windows x64, macOS (Apple silicon and Intel) and Linux x86_64, each with at least one tester |
| B4 | No data-loss incident, and no crash that loses work, in those 14 working days |
| B5 | Reading, filing, searching, forwarding and replying in rich text, with the quoted original kept |
| B6 | Accepting and declining a meeting; reminders fire as system notifications while MailCraft runs |
| B7 | Every account in step with Outlook on the web, checked by each tester at the end |
| B8 | Accounts of up to 200,000 messages: open under 3 s, a save under 1 s, under 1 GB of memory, measured on the reference machine and reported by testers |
| B9 | Sign-ins remembered on every beta platform |
| B10 | The Windows and macOS builds signed; the Microsoft registration publisher-verified |

## Scope

### In

1. **Hygiene first.** Fix the flaky saver test; correct the documents' stale facts; the clippy
   deny line in `crates/control-key`; the UIDVALIDITY fix (offline changes are re-applied where
   the message can still be found, and the owner is told of any that can't be); add
   `CONTEXT.md`.
2. **Remove Exchange Web Services.** `crates/net/src/ews*`, `FavoritesSource`, the choice in
   Add Account and File › Options, `oauth::refresh_ews` / `sign_in_for_ews`, and the rules in
   `CLAUDE.md`/`AGENTS.md` that name it. Favorites through Graph stay. An account whose
   Favorites were set to EWS falls back to Graph, or to MailCraft's own, and says so once.
3. **Graph mail for Microsoft accounts** ([ADR 0002](../adr/0002-graph-mail-for-microsoft-accounts.md)).
   Folders, messages and changes by Microsoft Graph; IMAP and SMTP are not used for Microsoft
   accounts. Every change an owner can make reaches the server or is refused with a reason:
   read, flag, categories, importance, move, copy, delete, folder create, rename, move and
   delete, drafts (kept in the server's Drafts). Undo of a change that reached the server is a
   reverse change sent to it. Imported mail goes to a local folder. An account synced by IMAP
   today is downloaded afresh by Graph: the switch says what it will do, keeps a backup of the
   mailbox file, and keeps local folders, drafts not yet on the server and MailCraft's own
   favourites.
4. **Rich-text reply, minimal.** The original's HTML quoted intact below; the owner's text with
   bold, italic, underline, bulleted and numbered lists, links and a font size; pasted
   formatting reduced to that set. Sent as HTML with a plain-text part.
5. **Sign-ins remembered on Linux** (Secret Service, behind `Services::secrets`, as on Windows
   and macOS).
6. **Bodies loaded on demand.** The mailbox keeps headers and the index in memory and reads a
   message's bodies and attachments from their files when they are needed; the first save after
   a large download runs in the background with progress. Measured on 2026-10-14 (Ryzen 5 5600X,
   NVMe): today 200,000 messages take 8.2 s to open and 4.7 GB of memory; the index alone is
   140 MB.
7. **System notifications** for reminders and new mail while MailCraft runs; **Copy
   diagnostics** (version, platform, the redacted log of what the program did; no message
   content, addresses, passwords or tokens; nothing sent by itself).
8. **The beta run**: recruitment, a GitHub issue template for testers, the checklist kept in
   `ROADMAP.md`.

### Out (until after beta)

IMAP accounts as a supported path (Gmail, Fastmail, Dovecot: the milestone after beta, starting
with a local Dovecot test server); rich composing beyond the minimal set (tables, styles, inline
pictures, spell check; sharing WordCraft's editor is its own decision); the per-message store
(gap 8 in full); notifications while MailCraft is closed; Propose New Time, rooms, Teams
meetings; `.pst`/`.msg`; languages; printing; S/MIME and OpenPGP; BSD and ARM as beta
platforms; a Discord channel for testers.

## How progress is reported

- **Measured numbers only**: catalog coverage, tests, platforms built, testers, days of use,
  data-loss incidents, open issues by label. No percentages that can't be reproduced.
- **Two documents**: `ROADMAP.md` (stage, the beta checklist with each item's state, the current
  focus, a progress log of one or two lines per entry) and `docs/gaps.md` (the ranked list).
  `docs/roadmap.md` and `docs/target-app-parity.md` are folded into them; the generated and
  per-area parity documents stay.
- **Testers report** in GitHub issues (the record), with Copy diagnostics attached. No telemetry.

## How the work is done

Mostly by agents, reviewed by a person. A change to sync (`crates/net`, `crates/proto`,
`crates/engine/src/cmd/account.rs`, `remote.rs`), the store (`crates/engine/src/store.rs`),
sign-in (`crates/net/src/oauth*`) or secrets (`apps/mailcraft/src/secrets.rs`) lands only by a
pull request a person has reviewed, enforced by `CODEOWNERS` and branch protection on `main`.

## Actions for the project owner

These need billsecond's accounts or money, and are B10:

- **Publisher verification** of the Microsoft registration (a Microsoft Partner Network id for
  the publisher), so testers in most organisations can sign in without an administrator.
- **Code signing**: an Apple Developer membership with notarisation for macOS; Azure Trusted
  Signing or a certificate for Windows.
- Accepting this proposal, ADR 0001 and ADR 0002; turning on branch protection and `CODEOWNERS`.

## Risks

| Risk | What we do |
|---|---|
| Graph throttles a first download of 200,000 messages | Batches, `Retry-After` honoured, progress shown, the download resumable; measured against the stand-in and the first testers |
| Publisher verification takes weeks | Recruit Outlook.com users and organisations whose administrator approves first |
| Graph mail is larger than estimated | It is the critical path; rich-text reply and Linux secrets run beside it |
| Testers don't come | Recruit from the Craft community before step 8, not at it |
| The project owner refuses part of this | Each step is a pull request of its own and can be taken or left alone |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-14 | major | First version, from a review of the repository and a decision session with Martin Ball |
| 2026-10-15 | minor | B2, B4: 14 working days rather than 14 calendar days, as a work mailbox is used (from the first tester sign-up, #33) |
