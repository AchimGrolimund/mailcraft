# Exchange servers other than Microsoft's

> **Last reviewed:** 2026-10-30 · **Last updated:** 2026-10-30 · **Change:** major

An Exchange server account (#252) reaches an organisation's own mail server through Exchange Web
Services. It is behind `MAILCRAFT_EWS=1` while it is tested. This page says what has been seen
working on servers other than Microsoft's, one at a time, and what has not. Everything here was
tried with made-up mail on a server running on the same computer; no real mailbox was used.

## Grommunio

**Seen:** Grommunio's own container deployment, Gromox 3.13.33 (October 2026), one domain, three
made-up mailboxes, TLS with a self-signed certificate, sign-in with Basic. The window itself was
not used for the answers below: the account was added and driven through the control channel (the
screen was locked), so the certificate question and the Add Account pages were not looked at
against this server.

| What | Result | Notes |
|---|---|---|
| Autodiscover | Reader works; the search was not run end to end | The server answers `/autodiscover/autodiscover.xml` with a Basic challenge, then with its Exchange Web Services address (`EXHTTP`, `EXCH`; the `EXPR` protocol has no address, and the `EXCH` server is a mailbox identifier, not a host). MailCraft reads that shape (a test keeps it). The test domain had no name that led to the server, so the three places Autodiscover looks were not tried from an address; the server's address was typed instead |
| Certificate by fingerprint | Works | A certificate the system does not trust ended the first run with the owner's question, and nothing was sent to the server meanwhile. With that one certificate's fingerprint trusted, every request went through, and the log has no line about the system's check any more (it used to note a failure on every request) |
| Sign-in: Basic | Works | |
| Sign-in: NTLM | Not offered | The server's challenge lists Basic only, so NTLM was not tried against a real server |
| First download | Works | Folders, then headers with a preview, then whole messages; an accented subject and body, an attachment and a message in a folder made on the server all arrived. The server's folders for calendar, contacts, tasks, notes, journal and settings are left out: Grommunio's Default listing gives no folder class, so MailCraft now asks for it by name and also leaves out the calendar, contacts, tasks and search elements whatever class they say (before this, they showed as mail folders) |
| Delta after a new message | Works | A message delivered after the first run came with the next one, and nothing came twice |
| Read, flag, categories | Work | Seen on the server afterwards. Importance of a message already received cannot be changed from here, so it was not tried |
| Move, copy | Work | Between folders made on the server |
| Delete | Works | To Deleted Items, or for good (`SoftDelete`) from Delete Permanently |
| Folders | Work | A folder made under Inbox, renamed, and deleted (it goes to Deleted Items) |
| Move to Archive | Works (fixed) | Grommunio has no Archive folder. When mail is put in the Archive of an Exchange account, MailCraft makes a folder named Archive at the top of the mail, once, and the move follows in the same run. A server that has such a folder already gives it to the Archive by its name. Tried again after the fix: one Archive on the server holding the message, and none made on later runs |
| Draft saved and edited | Works | One draft on the server, changed in place; High importance kept |
| Send to other local users | Delivered once | To and Bcc recipients each got it once |
| Bcc | Does not leak | The To recipient's copy has no Bcc, and the Bcc recipient got the message |
| Copy in Sent Items | Works (fixed) | Sending the saved draft with `SendItem` delivered it but filed no copy in Sent Items and left the draft in Drafts. Every send, a saved draft too, now goes as one `CreateItem` with `SendAndSaveCopy` and the message's own content, and the draft is then taken away with a soft delete, only after a clear success. Tried again after the fix: delivered once to the To and Bcc recipients, one copy in Sent Items, no draft left. **Not yet checked against Exchange Server itself**, which filed the copy for `SendItem` and is expected to do the same here |
| Undo of a move | Works | The message went back to where it was, on the server too |

### How sending keeps to "never twice"

The send is one request that is never repeated; an answer that is lost or unclear holds the message as "may have been sent" and the draft is kept. The draft is taken away only after a clear success. If that fails, the message is not sent again: the draft's removal waits for the next run as an ordinary delete.

## Not yet tried

Exchange Server 2019 and Subscription Edition, SmarterMail, NTLM against a real server, Kerberos
and modern sign-in (not built), a shared mailbox, and Autodiscover from an address.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-30 | major | First version: what was seen on Grommunio (#252, E8) |
| 2026-10-30 | minor | Sending a saved draft, the Archive folder, the folder class and the log line, fixed and tried again on Grommunio |
