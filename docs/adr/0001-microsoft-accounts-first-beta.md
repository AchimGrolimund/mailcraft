# 0001. Beta is for Microsoft accounts only

> **Status:** accepted (2026-10-14, [#1](https://github.com/UCS-Care-Group/mailcraft/pull/1)) · **Date:** 2026-10-14 · **Deciders:** billsecond, Martin Ball (co-owners)

## Context

MailCraft's milestone M1 said "a mainstream hosted account works for a week on two devices", and
its gap list put "run against real servers" first. In practice every connection that has met a
real server is a Microsoft 365 one: sign-in, Graph for sending, the calendar, contacts and
Favorites. Nothing has run against Gmail, Fastmail, Dovecot or a plain SMTP server. The project's
pitch is Outlook parity, so the people most likely to switch are on Microsoft accounts.

## Decision

Beta (as defined in `CONTEXT.md`) is for Microsoft accounts: personal ones (Outlook.com) and work
or school ones (Microsoft 365). IMAP accounts keep working as they do today but are not part of
the beta checklist, are not tested by beta testers, and become the milestone after beta.

## Alternatives

- **Any IMAP account first**: breadth, and a local Dovecot test server needs no credentials. But
  it means proving a second sync path before the first is proven, for people less likely to want
  an Outlook look-alike.
- **Both at once**: doubles the testing and the ways to lose data before beta.

## Consequences

- The beta checklist depends on Microsoft: publisher verification, Graph's limits and its
  consent rules are on the critical path.
- Gmail and other IMAP users wait until after beta; `docs/gaps.md` gap 1 moves below the line.
- The IMAP code is kept, tested by the fakes, and not extended until the milestone after beta.
