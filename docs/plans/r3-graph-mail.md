# R3: Graph mail for Microsoft accounts: implementation plan

> **Status:** proposed, with [the spec](../specs/r3-graph-mail.md) · **Written:** 2026-10-15 · **Change:** major (first version)

Refines step R3 of [the roadmap reset plan](roadmap-reset.md). Requirement numbers (S1–S36) are
the spec's. Each task is one pull request, reviewed by a person (`needs-review`), with
`cargo xtask ci` green on all three CI platforms; task ids go in commit messages
(`R3.2: messages come down by delta`). The Graph stand-in (`crates/net/src/graph/standin.rs`)
grows inside each task with exactly what that task needs. The engine stays free of I/O: the
request/apply split of `crates/model/src/sync.rs` gains Graph types of its own beside the IMAP
ones, rather than bending the uid-based ones. Hours are guesses until a task is done.

## R3.1 Folders, and the ground under everything else (#12) (14–22 h)

| Part | What |
|---|---|
| Account kind | `AccountKind::Microsoft` (S1); behind the testing switch (S35); Add Account makes one only with the switch on |
| Server ids | A server id on `Message` and `Folder`, a delta link per folder, and the last synced state of a message widened to read, flag, categories, importance and folder (ADR 0003); older files open with these empty |
| Graph client | `Prefer: IdType="ImmutableId"` on every request (S4); the retry policy by kind of request, `Retry-After`, a per-account pause and at most 4 requests at once, 2 for background work (S14, S21) |
| Folders | The full walk of mail folders, hidden and search folders left out, Outlook's order (S6); New, Rename, Move, Delete Folder to Deleted Items (S7); removal only after a complete listing (S8) |
| Stand-in | Folders: list, childFolders, create, PATCH, move, delete, names with `/` and `\\`; 429 with `Retry-After` |

**Done when:** stand-in flow tests both ways (a folder made here is there; one renamed, moved or
deleted there is so here, keeping its server id and its messages); folders named `Sales/Marketing` and
`A\\B` are listed under their parent, whole; a listing cut off removes
nothing; a 429 pauses and resumes; a mailbox file from 0.2.1 opens.

## R3.2 Mail down (#13) (16–26 h)

| Part | What |
|---|---|
| Sign-in | The permissions of S2, asked again of accounts signed in before; admin consent list; `CLAUDE.md`, `AGENTS.md`, `PRIVACY.md` |
| Delta | Per folder, newest first, headers and preview (S9); next-page and delta links stored after each page (S10); expired links listed afresh (S11); taken in by server id (S5) |
| Fetching | MIME in the background newest first, at once on opening, before Reply, Forward and Save (S12); offline wording; search counts what isn't searched in full (S13) |
| Progress | Per folder and overall; Cancel after the page in hand (S15) |
| Stand-in | Message delta with pages, `@removed`, a replayed page, `410` and `syncStateNotFound`; `$value`; a generated 200,000-message mailbox |

**Done when:** 200,000 messages from the stand-in, interrupted and resumed three times at random,
arrive with nothing twice and nothing missing; the list is usable before the fetch ends; an
expired link relists that folder and removes only what a complete listing lacks.

## R3.3 Changes up (#14) (14–22 h)

| Part | What |
|---|---|
| Differences | From the last synced state: read, flag, categories, importance as one PATCH, a different folder as a move (S16); the conflict rule (S17); categories by name, colours read (S18) |
| Queue | Copy and delete for good (S16) |
| Deleting | Delete to Deleted Items; delete for good with Graph's delete; never `permanentDelete`; moving to a local folder fetches, copies, then removes (S19) |
| Refused | Put back to the server's state, said once, logged (S20); a move or copy with a lost answer looked up before retrying (S21) |
| Stand-in | PATCH, move, copy, delete; 404/403/400 on demand; a connection dropped after the request |

**Done when:** one test per kind of change; a refused change is put back and said; a change made
here and one made on the server to the same message end as S17 says; a grep test finds no
`permanentDelete` anywhere.

## R3.4 Drafts and sending (#15) (12–18 h)

| Part | What |
|---|---|
| Drafts up | Created with our extended property, found again after a lost answer (S23); create-reply and create-forward; changed in place field by field, attachments added and removed, large ones by upload session (S24); on save, close and every 5 minutes (S25); edited on both sides kept twice (S26) |
| Sending | Send-draft or `sendMail`, no SMTP fallback, no copy uploaded (S27); held messages (S28) |
| Stand-in | Create from MIME and JSON, createReply/createForward, PATCH of drafts, attachments and upload sessions, send, `sendMail`, `$filter` by extended property |

**Done when:** a draft written here is in the stand-in's Drafts and back, once, after a lost
answer; editing it changes it in place; a reply stays in its conversation; a send whose answer is
lost is held and not sent again.

## R3.5 Undo (#16) (4–8 h)

Undo for a Microsoft account no longer forgets its history when a change reaches the server; the
next run sends the reverse (S29). **Done when:** undo of a move, a flag, categories and a delete
reach the stand-in as their reverses; undo of a delete for good says it can't.

## R3.6 Imported mail (#17) (2–4 h)

Imports go to a local folder; moving imported mail into a server folder is refused with the reason
(S30). **Done when:** tested; the refusal's wording is in the UI test.

## R3.7 The switch (#18) (10–16 h) [owners read the wording]

The once-only page; the last IMAP push first, waiting when offline; the dated backup; the fresh
download; local folders, unsent drafts and favourites kept (S31–S33). The testing switch goes
(S35). **Done when:** a mailbox from 0.2.1 with an offline flag, an offline move, a local folder,
an unsent draft and a favourite switches against the fake IMAP server and the stand-in, and keeps
all five.

## R3.8 IMAP off for Microsoft accounts (#19) (4–6 h)

When S34's measure is met: the XOAUTH2 IMAP and SMTP path for Microsoft accounts, its permissions
and the switch's IMAP half go. **Done when:** no IMAP or SMTP connection is made for a Microsoft
account in any flow test; the admin consent list has no IMAP or SMTP permission.

## R3.9 The author's mailbox, 7 days (by hand)

Martin Ball's own Microsoft 365 mailbox on a build with R3.1–R3.7, daily, for 7 days, checked
against Outlook on the web at the end. Any data-loss incident is fixed with a regression test and
the 7 days restart.

## Order

```
R3.1 ─ R3.2 ─ R3.3 ─ R3.4 ─ R3.5 ─ R3.7 ─ R3.9 ─ R3.8
                     └─ R3.6 ┘
```

Total: about 76–122 hours before R3.9.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version |
