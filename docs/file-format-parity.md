# File formats and protocols

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-11 · **Change:** major (first version) · **Target:** Microsoft Outlook (Microsoft 365, classic desktop)

What MailCraft reads, writes and speaks, and how far each goes. Everything is implemented from the
public specifications; every reader is fed arbitrary bytes by property tests and must never panic.

## Files

| Format | Read | Write | Notes |
|---|---|---|---|
| **`.mailcraft`** (the mailbox, JSON) | yes | yes | Everything in one file; `repair()` makes any file consistent. Will be replaced by a per-message store (gap #8) |
| **`.eml`** (RFC 5322 + MIME) | yes | yes | Multipart trees, encoded words, RFC 2231 names, quoted-printable and base64, attachments and inline parts, importance, keywords, status flags; round-trips its own output. No uuencode, TNEF (`winmail.dat`), `format=flowed` |
| **`.mbox`** (RFC 4155, mboxrd-tolerant) | yes | yes | `Content-Length` variants not honoured |
| **`.ics`** (iCalendar, RFC 5545) | yes | yes | Events, to-dos, attendees, alarms, recurrence (the common rules), skipped days, changed instances read as separate events. No VTIMEZONE (gap #10), `RDATE`, recurring to-dos |
| **`.vcf`** (vCard 2.1 / 3.0 / 4.0) | yes | 3.0 | Names, organisation, e-mails, phones, addresses, URLs, birthday, notes, categories. No photos, groups, name prefixes/suffixes |
| `.msg`, `.pst`, `.ost`, `.olm`, `.oft` | no | no | Gap #12 |
| `.html`, `.pdf` (save a message as) | no | no | With printing (gap #15) |

Text encodings: UTF-8, US-ASCII, ISO-8859-1/15 and Windows-1252 are decoded properly; others
lossily (gap #13). Output is always UTF-8.

## Protocols

| Protocol | State | Notes |
|---|---|---|
| **SMTP submission** (RFC 5321, 4954, 3207) | send | EHLO/HELO, STARTTLS or TLS from the start, AUTH PLAIN and LOGIN, dot-stuffing, multi-line replies. No OAuth, SMTPUTF8, DSN, pipelining |
| **IMAP4rev1** (RFC 3501, special-use 6154) | download | LOGIN, LIST, EXAMINE, UID FETCH (flags, sizes, whole messages with literals), modified UTF-7 names, UIDVALIDITY handling; `uid_store` and `append` exist but sync doesn't use them yet (gap #2). No IDLE, CONDSTORE/QRESYNC, AUTHENTICATE, COMPRESS, partial fetch |
| **TLS** | rustls with the Mozilla root set | Certificate and host name verified; unencrypted connections refused except to loopback; a missing STARTTLS offer is an error |
| POP3 | no | Not planned before M6 |
| Exchange (EWS, MAPI), Microsoft Graph, JMAP, CalDAV, CardDAV | no | Gap #5; JMAP and Graph are how the browser build will get a server connection |

Interoperability with real servers has not been tested (gap #1).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-11 | major | First version |
