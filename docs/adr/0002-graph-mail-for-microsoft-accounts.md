# 0002. Microsoft accounts read and change mail through Microsoft Graph only

> **Status:** accepted (2026-10-14, [#1](https://github.com/billsecond/mailcraft/pull/1)) · **Date:** 2026-10-14 · **Deciders:** billsecond, Martin Ball (co-owners)

## Context

A Microsoft account's mail comes down by IMAP (XOAUTH2) today; sending, the calendar and contacts
already go through Microsoft Graph. Many organisations switch IMAP off in Microsoft 365, so their
people can't connect at all. Two-way sync over IMAP still leaves folder renames, moves and
deletes, drafts, copies, categories and importance unsent (gap 2), and categories and importance
have no IMAP equivalent Outlook reads. The browser build can't open sockets, so IMAP can never
serve it; Graph is HTTPS.

## Decision

For Microsoft accounts, folders, messages and every change to them go through Microsoft Graph.
IMAP and SMTP are not used for them, with no fallback. Every change an owner can make reaches the
server or is refused with a reason, and undoing a change that reached the server is a reverse
change sent to it. An account synced by IMAP before this is downloaded afresh: the program says
what it will do first, keeps a backup of the mailbox file, and keeps local folders, unsent drafts
and MailCraft's own favourites.

## Alternatives

- **IMAP for beta, Graph after**: less work now; leaves organisations with IMAP off unable to
  connect, and gap 2 to finish over a protocol that can't carry categories.
- **Graph with IMAP as fallback**: two sync paths to prove, and a fallback that hides which one
  ran when something goes wrong.
- **Match the IMAP copies to Graph's messages** (by Message-ID) instead of downloading afresh:
  keeps local state for the one mailbox that has any, at the cost of matching code used once.

## Consequences

- The Graph sync is the largest piece of work before beta and its critical path.
- Testers in organisations that block IMAP can take part.
- The browser build gets a server connection for Microsoft accounts later at little cost
  (M6), subject to Graph's cross-origin rules.
- The IMAP code for Microsoft accounts (XOAUTH2 for IMAP and SMTP) becomes unused and can be
  removed once the switch is proven; the IMAP code for other accounts stays.
- A first download of a large mailbox is bound by Graph's throttling; it has to resume and show
  progress.
