# New-mail notifications on iPad: MailCraft relay

iPadOS doesn't let an app keep a connection to a mail server open in the background, so MailCraft
on iPad hears of new mail only while it is open, unless something tells it. **MailCraft relay**
does that from a computer of yours that is always on (Windows, a Mac or Linux): it watches your
Inboxes and, when unread mail arrives, sends your iPad a notification through Apple's push
service. Nobody else's server is involved, and it costs nothing beyond the Apple developer
account the iPad app is built with.

## What goes where

- **Your sign-ins and passwords** stay on the computer, in its credential store (Windows
  Credential Manager, the macOS keychain, the Secret Service on Linux), and go only to your mail
  servers. Gmail and Google Workspace sign in on Google's page, Outlook.com and Microsoft 365 on
  Microsoft's: the relay keeps the sign-in (a refresh token you can revoke in your Google or
  Microsoft account at any time), never a password. Any other account asks for its password
  (an app password where the provider offers one).
- **The relay reads nothing of your mail** but which messages are new and unread in the Inbox. It
  opens the Inbox read-only: nothing is marked read, moved or changed.
- **A notification says only** which account has new mail and how many messages ("3 new
  messages"): never a sender or a subject, which Apple would see.
- **Your push key** (from your Apple developer account) is kept in the computer's credential
  store too.
- **Pairing** happens once, on your home network: the iPad sends the relay its push address and
  the code the relay shows. After that, notifications reach the iPad anywhere.

## Setting it up

Google sign-in needs the build to have a Google OAuth client of its own (`MAILCRAFT_GOOGLE_CLIENT_ID`
and `MAILCRAFT_GOOGLE_CLIENT_SECRET`, repository secrets that the Relay and iPad workflows build
in): a *Desktop app* client in Google Cloud whose consent screen is *In production* (one left in
*Testing* has its sign-ins expire after seven days), with the scope `https://mail.google.com/`.

1. **The push key.** In your Apple developer account, Certificates, IDs & Profiles › Keys › **+**:
   give it a name, tick **Apple Push Notifications service (APNs)**, choose *Production* (or
   *Sandbox & Production*), and download the `AuthKey_XXXXXXXXXX.p8` file. Note its **Key ID**,
   and your **Team ID** (top right of the developer site). The same App ID must have *Push
   Notifications* turned on.
2. **The relay.** Download `mailcraft-relay-windows-x64` from the *Relay* workflow's latest run on
   GitHub and unzip it anywhere (keep both programs together). Open a terminal there:

   ```
   mailcraft-relay apple-key AuthKey_XXXXXXXXXX.p8 --key-id XXXXXXXXXX --team-id XXXXXXXXXX
   mailcraft-relay add you@gmail.com
   mailcraft-relay add you@outlook.com
   ```

   `add` opens Google's or Microsoft's sign-in page in the browser (or, for any other account,
   asks for the password, not shown as you type), and checks that the Inbox opens. A Google
   Workspace address at a domain of its own takes `--google`. For a server it doesn't know, add
   `--server imap.example.com[:993]` (and `--user name` when that isn't the address).
3. **Pair the iPad.** Run `mailcraft-relay pair`. It shows this computer's address and a six-digit
   code. On the iPad: File › Options › **Push from your computer**, enter both and tap **Pair**.
   Windows may ask whether the relay may use the network: allow private networks. iPadOS asks
   whether MailCraft may find devices on your local network, and may show notifications: allow
   both.
4. **Start it with Windows.** `mailcraft-relay install` starts it whenever you sign in to Windows,
   without a window, and starts it now. (On a Mac or Linux, have `mailcraft-relay run` started at
   login.) `mailcraft-relay test` sends a test notification.

Run `add` and `apple-key` at the computer itself (or over Remote Desktop), not over SSH: Windows
doesn't let a session signed in with an SSH key use the credential store.

`mailcraft-relay status` shows the accounts, the paired iPads, whether it is running and the end
of its log (`relay.log` in `%APPDATA%\MailCraft Relay`). `mailcraft-relay remove you@example.com`
stops watching an account and forgets its password; `mailcraft-relay uninstall` stops it starting
with Windows.

## How it works

Each account has a connection that waits with IMAP IDLE (RFC 2177), so the server says at once
when mail arrives; a server without IDLE is asked once a minute. The connection is renewed every
nine minutes, a broken one is made again (waiting longer each time, up to 15 minutes), and mail
that arrived while the relay was off is announced once when it starts again. Notifications go
to Apple over HTTP/2 with a token signed by your push key (renewed every 40 minutes). An iPad
that removed the app is unpaired by itself.
