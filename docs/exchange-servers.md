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
| Certificate by fingerprint | Works | A certificate the system does not trust ended the first run with the owner's question, and nothing was sent to the server meanwhile. With that one certificate's fingerprint trusted, every request went through. The log notes the system's check failing on each request before the fingerprint is used |
| Sign-in: Basic | Works | |
| Sign-in: NTLM | Not offered | The server's challenge lists Basic only, so NTLM was not tried against a real server |
| First download | Works | Folders, then headers with a preview, then whole messages; an accented subject and body, an attachment and a message in a folder made on the server all arrived. The server's own folders for calendar, contacts, tasks, notes and the like are left out |
| Delta after a new message | Works | A message delivered after the first run came with the next one, and nothing came twice |
| Read, flag, categories | Work | Seen on the server afterwards. Importance of a message already received cannot be changed from here, so it was not tried |
| Move, copy | Work | Between folders made on the server |
| Delete | Works | To Deleted Items, or for good (`SoftDelete`) from Delete Permanently |
| Folders | Work | A folder made under Inbox, renamed, and deleted (it goes to Deleted Items) |
| Move to Archive | **Does not go up** | Grommunio has no Archive folder, and the one MailCraft shows stays on this computer: the message moves there, the server keeps it in the Inbox, and Send/Receive says it is up to date |
| Draft saved and edited | Works | One draft on the server, changed in place; High importance kept |
| Send to other local users | Delivered once | To and Bcc recipients each got it once |
| Bcc | Does not leak | The To recipient's copy has no Bcc, and the Bcc recipient got the message |
| Copy in Sent Items | **Not made for a saved draft** | Sending the saved draft delivers it, but Grommunio neither files a copy in Sent Items nor takes the draft out of Drafts, whatever `SendItem` is told. It stays in Drafts and Sent Items is empty. A message never saved as a draft goes as one request that asks for a copy, and Grommunio files that one (tried by hand, not through the window) |
| Undo of a move | Works | The message went back to where it was, on the server too |

### Not fixed

- A saved draft that has been sent stays in Drafts and no copy is filed in Sent Items on
  Grommunio. Exchange files it in Sent Items itself; a fix would move the draft there when the
  server has left it behind, and needs a run against Exchange to be sure it does not file twice.
- A message moved to Archive on a server that has none stays on this computer without a word.

## Not yet tried

Exchange Server 2019 and Subscription Edition, SmarterMail, NTLM against a real server, Kerberos
and modern sign-in (not built), a shared mailbox, and Autodiscover from an address.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-30 | major | First version: what was seen on Grommunio (#252, E8) |
