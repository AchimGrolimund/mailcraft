# R3: Graph mail for Microsoft accounts

> **Status:** proposed · **Written:** 2026-10-15 · **Author:** Martin Ball, with Claude · **Change:** major (first version) · **Target:** Microsoft Outlook (Microsoft 365, Outlook.com)

The words in this document have the meanings in [`CONTEXT.md`](../../CONTEXT.md). It details step
R3 of [the roadmap reset](roadmap-reset.md); the decisions behind it are
[ADR 0002](../adr/0002-graph-mail-for-microsoft-accounts.md) (Graph only, no IMAP fallback) and
[ADR 0003](../adr/0003-server-ids-and-changes-from-the-last-synced-state.md) (server ids, changes
from the last synced state, deleting as Outlook does). The order of the work is in
[`docs/plans/r3-graph-mail.md`](../plans/r3-graph-mail.md). Tracked in issues #12–#19.

## Why

Beta needs every Microsoft account in step with Outlook on the web (B7), and organisations that
switch IMAP off able to connect at all. Today, on `main`:

- Graph sends mail and handles the calendar, contacts, Favorites and working hours; **mail itself
  comes down and goes up only by IMAP**.
- An account is `Local` or `Imap`; the sync request refuses anything else.
- A message is known by its IMAP uid and Message-ID, a folder by its path. There is no field for a
  server id.
- The sign-in doesn't ask for `Mail.ReadWrite`.
- The HTTP client opens a connection per request and retries nothing; only the Favorites reader
  honours `Retry-After`. `sendMail` has no protection against being sent twice.
- Categories, importance and copy have no way up; drafts never go up; undo forgets its history once
  a change reached the server; imported mail lands in the current folder and never goes up.
- The Graph stand-in fakes no mail folders beyond one read, no messages, no delta, no sending.

## Goal

A Microsoft account's folders and mail are kept in step through Microsoft Graph alone: everything
the owner does in MailCraft happens on the server as it would in Outlook, every change reaches the
server or is refused with a reason, nothing comes down twice and nothing is missed, and accounts of
up to 200,000 messages are usable within minutes of being added.

## Requirements

### Accounts and sign-in

1. A Microsoft account is a kind of account of its own (not an IMAP account with a marker). It has
   no incoming or outgoing server settings; it is kept in step through Graph and never by IMAP or
   SMTP.
2. A sign-in asks for `Mail.ReadWrite`, `Mail.Send`, `Calendars.ReadWrite`, `Contacts.Read`,
   `MailboxSettings.Read`, `offline_access`, `openid`, `profile` and `email`. `Mail.ReadBasic` is
   no longer asked for (`Mail.ReadWrite` covers it). An account signed in before is asked once
   more, through the existing flow. The administrator consent list matches. The IMAP and SMTP
   permissions are dropped at R3.8. The sign-in rule in `CLAUDE.md` and `AGENTS.md` and
   `PRIVACY.md` change with it; this is an owners' decision, agreed by Martin Ball on 2026-10-15
   and to be agreed by billsecond in the pull request.
3. Shared mailboxes and delegate access are not part of R3 (an issue after beta). The server id of
   a folder is stored so that a mailbox's address can be added beside it later.

### Identity

4. Every Graph request asks for immutable ids (`Prefer: IdType="ImmutableId"`). A message is known
   by its immutable id, which stays the same when it moves between folders of the mailbox; a folder
   by its Graph id, which stays the same when it is renamed or moved. A folder's path is for
   showing only. The Message-ID is kept and never trusted to say two messages are the same.
5. Everything taken in from the server is matched by server id, so a delta page that arrives twice
   (Microsoft warns that it can) changes nothing.

### Folders (R3.1)

6. The folder list is read in full each run by walking every mail folder's subfolders (folder delta
   isn't documented to cover every depth; folders are few). Hidden folders and search folders are
   not shown. Order: the well-known folders first, as Outlook orders them, then the rest
   alphabetically, nested.
7. New Folder, Rename, Move Folder reach the server. Delete Folder moves the folder under Deleted
   Items, as Outlook does.
8. A folder the complete listing no longer has is removed here with its mail, which the server no
   longer has either. A listing that didn't finish removes nothing.

### Mail down (R3.2)

9. Each folder is read by Graph's message delta, newest first (`$orderby=receivedDateTime desc`),
   in pages, asking for the headers, the preview, read, flag, categories, importance, the folder
   and the Message-ID: enough for the message list. Deletions arrive as `@removed`.
10. After each page is taken in and saved, the folder's next-page link (or, at the end, its delta
    link) is stored with the folder in the mailbox file. Cancel, quitting, a crash or going offline
    carry on from there next time: nothing twice, nothing missed.
11. A delta link the server no longer accepts (`410 Gone` or `syncStateNotFound`) means that folder
    is listed afresh; what the complete new listing doesn't have is then removed here.
12. A message's whole content (its MIME, with attachments) is **fetched** in the background, newest
    first across the account, and at once when the owner opens it. Reply, Forward and saving a
    message fetch it first. Opening a message that isn't fetched while offline says it isn't on
    this computer yet and will download when online.
13. Search covers headers and previews of every message, and the whole text of fetched ones; while
    the background fetch isn't finished, results say how many messages weren't searched in full.
14. Microsoft allows 4 requests at once per mailbox. Background fetching uses at most 2; what the
    owner is waiting for (opening a message, sending, Send/Receive) goes first. A "busy" answer
    (429, 503 with `Retry-After`) pauses all Graph work for that account for as long as it asks.
15. Progress is shown per folder and overall, and Cancel stops after the page in hand.

### Changes up (R3.3)

16. Each message keeps what the server last said of it: read, flag, categories, importance and
    the server folder it is in. A run sends the difference between that and the message as it is
    now, per property: read, flag, categories and importance as one change of the message, a
    different folder as a move. A copy and a delete for good wait in a queue (they can't be worked
    out by comparing).
17. Where a message changed here and on the server since the last run, what was changed here wins
    for the properties it touched; the server's version for the rest.
18. Categories are applied by name. Their colours are read from the master list; a category made
    here shows in Outlook without a colour until it is given one there. The master list is not
    written in R3.
19. **Deleting is Outlook's** (ADR 0003): Delete moves to Deleted Items; Delete for good (Shift+Del,
    emptying Deleted Items) removes from the folders with Graph's delete, recoverable for the
    server's retention period; Graph's `permanentDelete` is never used. Moving a server message into
    a local folder fetches it whole, copies it there, and then removes it from the server.
20. A change the server refuses (404 gone, 403 not allowed, 400 invalid) is put back to how the
    server has it, said once in the Send/Receive status in plain words ("Couldn't move 'Invoice 42'
    to Archive: the server says it no longer exists.") and written in full to the Send/Receive log.
    A busy server or a lost connection is not a refusal: the change waits for the next run.

### Retrying and never twice (all steps)

21. One retry policy for all of Graph: reads (GET) and changes that are the same however often they
    are made (setting read, flag, categories, importance; delete) are retried with back-off,
    honouring `Retry-After`. A move or copy whose answer was lost is retried only after looking
    where the message is now. `sendMail` and sending a draft are never retried by themselves.
22. Connections stay one per request; `$batch` (up to 20 requests) is used where it saves round
    trips. Revisited only if measurement against the stand-in says so.

### Drafts and sending (R3.4)

23. A draft saved here is created in the server's Drafts carrying an extended property of ours, so
    a create whose answer was lost is found again instead of made twice. A reply or forward draft is
    created with Graph's create-reply/create-forward, so it stays in its conversation.
24. Saving again changes the server's draft in place: subject, body (HTML), To, Cc, Bcc and
    importance changed field by field; an attachment added here is uploaded (directly under 3 MB,
    else through an upload session, up to the server's message size limit), one removed here is
    removed. Headers such as `In-Reply-To` and `References` go in `internetMessageHeaders` where
    Graph allows.
25. Drafts go up when the owner saves or closes, and every 5 minutes while one is open.
26. A draft edited both here and elsewhere since the last run is kept twice: the server's version
    comes down as a second draft, nothing lost.
27. A draft on the server is sent with Graph's send-draft action; a message never saved as a draft
    with `sendMail`. The server files the copy in Sent Items; it comes down by delta, and MailCraft
    uploads no copy of its own. No SMTP, no fallback.
28. When it isn't known whether a message went (the answer was lost), or the server refused it, it
    becomes a **held message**: kept in the Outbox, not sent again by itself, with what happened
    and what to check ("may have been sent: look in Sent Items").

### Undo (R3.5)

29. Undo restores the mailbox as today; the next run sends the difference, which is the reverse
    change (a move back, a flag cleared, out of Deleted Items). The undo history is no longer
    forgotten when a change reaches a Microsoft account's server. Undo of a delete for good says it
    can't be undone here.

### Imported mail (R3.6)

30. Graph cannot put received mail into a mailbox folder as an ordinary message (what it creates is
    always a draft; its import API is for administrators, not for personal accounts, and not for
    MIME). Imported mail (`.eml`, `.mbox`) therefore goes to a local folder, and moving it into a
    server folder is refused with the reason: "Microsoft doesn't let a mail program put received
    mail into your mailbox; it stays in a local folder."

### Switching an account synced by IMAP (R3.7)

31. Once the switch has landed, each Microsoft account still on IMAP shows a page once: what will
    happen, with Switch and Later.
32. Switch: a last IMAP run pushes what is waiting first, and the switch waits if it can't
    (offline); the mailbox file is backed up beside itself with the date in its name; server folders
    and their messages are dropped and downloaded afresh through Graph; local folders are kept;
    drafts not on the server are uploaded as server drafts; MailCraft's own favourites are matched to
    the new folders by path.
33. Later keeps that account on IMAP until R3.8.

### Turning IMAP off for Microsoft accounts (R3.8)

34. IMAP and SMTP for Microsoft accounts are removed (code, permissions, the switch's IMAP half) when
    the author's mailbox has run on Graph for 7 days without a data-loss incident (R3.9) and at
    least 2 alpha testers have switched and used it for 7 days without one.

### Until R3 is whole

35. A Microsoft account becomes a Graph account only behind a switch for testing (an environment
    variable, or a hidden option) until R3.7 is merged; released builds stay on IMAP until then.
    Each task is one pull request, reviewed by a person (`needs-review`).

### Tests

36. Every done-when is proved against the Graph stand-in, which grows with each task: the folder
    walk and folder changes; message delta in pages with `@removed`, replayed pages and both kinds
    of expired link; MIME; message changes, move, copy, delete; create from MIME, create-reply,
    draft PATCH and attachments, send-draft, `sendMail`; filtering by extended property; 429 with
    `Retry-After`; a connection that drops after the request went (for held messages); and a
    generated mailbox of 200,000 messages. No test touches a real mailbox; R3.9 is the author's own,
    by hand.

## Out of scope

Shared mailboxes and delegates; writing the category master list; a Sync Issues folder; push
notifications from Graph (the Inbox is looked at on the Send/Receive timer); IMAP accounts (the
milestone after beta); importing into server folders.

## Risks

| Risk | What we do |
|---|---|
| The background fetch of 200,000 messages takes hours under Graph's limits | The list is usable from headers in minutes; fetch newest first; measured against the stand-in and the author's mailbox |
| Graph's undocumented corners (where a plain delete goes, `$batch` with MIME, folder delta depth) | Requirements avoid relying on them; R3.9 observes the real service |
| Delta links expire unpredictably | A complete fresh listing per folder, removing only what a whole listing lacks |
| A send whose answer is lost | Held, never resent by itself |
| The permission change needs administrators to approve again | Said by the existing approval flow; the same consent list covers it |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version, from a decision session with Martin Ball |
