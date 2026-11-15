# New-mail notifications on iPad: MailCraft relay

iPadOS doesn't let an app keep a connection to a mail server open in the background, so MailCraft
on iPad hears of new mail only while it is open, unless something tells it. **MailCraft relay**
does that from a computer of yours that is always on (Windows, a Mac or Linux): it watches your
Inboxes and, when unread mail arrives, sends your iPad a notification through Apple's push
service. Nobody else's server is involved, and it costs nothing beyond the Apple developer
account the iPad app is built with.

## What goes where

- **Your accounts come from the iPad.** Once the iPad is paired, every IMAP account on it (with
  its password, or for Gmail, Google Workspace, Outlook.com and Microsoft 365 its sign-in: a
  refresh token you can revoke in your Google or Microsoft account at any time) goes to the relay
  whenever it changes, sealed, on your home network; an account you remove on the iPad is dropped
  there with its sign-in. Away from home, the change waits until the iPad is back. Accounts can
  also be added on the computer itself (`mailcraft-relay add`); those stay as they were set up.
- **Your sign-ins and passwords** are kept on the computer in its credential store (Windows
  Credential Manager, the macOS keychain, the Secret Service on Linux), never in a file, and go
  only to your mail servers.
- **Each device gets its own.** The relay keeps, for each paired iPad, the accounts it brought,
  and notifies it of those alone (and of accounts added on the computer, which every device gets).
  An account no device wants any more is no longer watched, and its sign-in is forgotten.
- **The relay reads nothing of your mail** but which messages are new and unread in the Inbox,
  and the sender and subject of the newest one. It opens the Inbox read-only: nothing is marked
  read, moved or changed.
- **A notification is sealed for your iPad.** Apple carries "New mail" and nothing else it can
  read: the account, the sender and the subject are sealed (ChaCha20-Poly1305) with a key the iPad
  made and gave the relay when it paired, and MailCraft's notification extension opens them on the
  iPad. Notifications of one account replace one another, and Apple drops one it couldn't deliver
  within a day.
- **Your push key** (from your Apple developer account) is kept in the computer's credential
  store too.
- **Pairing** happens once, on your home network, with the six-digit code the relay shows. The code
  never crosses the network: both ends prove they know it with SPAKE2, a password-authenticated
  key exchange, so somebody who records the exchange can't try codes at leisure, and somebody who
  tries codes gets five tries an hour. Pairing leaves a key on both ends with which every later
  exchange is sealed and authenticated, with fresh keys each time (X25519): only your paired iPad
  can send the relay accounts, and only your relay can read them.

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
3. **Pair the iPad.** `add` ends by showing this computer's address and the six-digit pairing
   code, and `mailcraft-relay code` shows them at any time (`code --new` makes a new code). On
   the iPad, on the same home network: File › Options › **Push from your computer**, enter both
   and tap **Pair**. The running relay listens for iPads for as long as it runs (on the home
   network only; after five pairing tries in an hour it takes none until the hour is up), and
   `mailcraft-relay pair` says when an iPad has paired. The iPad talks to this computer only on
   the home network (to pair, and to bring its accounts and its push address up to date):
   notifications reach it anywhere, through Apple, and nothing on this computer is open to the
   internet. Options on the iPad then shows each account the relay watches for it, and what is
   wrong with any it can't.
   Windows may ask whether the relay may use the network: allow private networks. iPadOS asks
   whether MailCraft may find devices on your local network, and may show notifications: allow
   both.
4. **Start it with Windows.** `mailcraft-relay install` starts it whenever you sign in to Windows,
   without a window, and starts it now. (On a Mac or Linux, have `mailcraft-relay run` started at
   login.) `mailcraft-relay test` sends a test notification.

Run `add` and `apple-key` at the computer itself (or over Remote Desktop), not over SSH: Windows
doesn't let a session signed in with an SSH key use the credential store.

`mailcraft-relay status` shows the accounts, the paired iPads, whether it is running and the end
of its log (`relay.log` in `%APPDATA%\MailCraft Relay`). `mailcraft-relay devices` lists the
paired iPads with the accounts each wants, and `mailcraft-relay unpair <n>` forgets one (Forget on
the iPad tells the relay itself). `mailcraft-relay remove you@example.com` stops watching an
account added on the computer and forgets its password; `mailcraft-relay uninstall` stops it
starting with Windows. An iPad paired by an earlier version of MailCraft is asked to pair again.

## How it works

Each account has a connection that waits with IMAP IDLE (RFC 2177), so the server says at once
when mail arrives; a server without IDLE is asked once a minute. The wait is renewed every nine
minutes on the same connection (a server that says nothing meanwhile, as Gmail does, is no
reason to reconnect); a broken connection is made again, waiting longer each time (up to 15
minutes, and an hour after the server refused the sign-in, so that a changed password doesn't
get the account locked), and mail that arrived while the relay was off is announced once when it
starts again. Accounts that iPads add or drop are started and stopped while the relay runs.
Notifications go to Apple over one HTTP/2 connection kept open between them, with a token signed
by your push key (renewed every 40 minutes). The iPad sends its push address each time MailCraft
starts at home, so a new one Apple gives it (after a restore, say) reaches the relay; an iPad that
removed the app is unpaired by itself.
