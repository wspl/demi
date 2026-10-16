# Users and instance mode

Who uses a Demi server and whose providers they infer with: the instance mode, and the user system of accounts, roles and sign-in.

## Instance mode: shared vs isolated

`DEMI_INSTANCE_MODE` selects whose provider entries a user infers with. The
product cannot change this deployment setting. Every entry belongs to a user;
a shared instance has no separate instance-owned kind of entry. Its providers
are the master's, and only the master configures them. Entries a user made
under the other mode stay theirs and are simply not the ones the mode selects.

| Mode | Provider configuration | Model use | Usage visibility |
|---|---|---|---|
| Shared | The master manages their own entries | All users use the master's entries | Each user sees their own usage; admins can see instance usage by user |
| Isolated | Each user manages their own entries | Each user uses their own entries | Each user sees their own usage |

Conversation, device, workspace, and attachment ownership remains per user in
both modes. Administrator status does not grant access to another user's
conversations or files. Shared usage reporting and account administration are
explicit exceptions to owner-only product records.

A provider is a configured entry with a stable ID and label. Model selection uses
`(providerId, modelId)`, so two entries can offer the same vendor model under
different accounts or endpoints. Subscription entries can contain multiple
accounts with one selected account. Credential rules belong to
[Providers](../providers/providers.md), and catalog rules to
[Models](../providers/models.md).

## User system

Accounts use email and password; nickname is a separate display value. The first
account is created by instance setup. Administrators create subsequent accounts;
there is no public registration or password-recovery flow.

| Role | Account administration |
|---|---|
| Master | Creates admins and users; resets passwords of admins and users |
| Admin | Creates users; resets user passwords |
| User | No administration of other accounts |

A role can act only on lower roles. No role can reset a peer's password through
the admin endpoint.

A password an administrator chose is the account's only until its owner
chooses one. An account made by Add User…, and one whose password an
administrator reset, chooses its own password the next time its owner signs
in, by that password on a computer or by a sign-in code on a phone: before
anything else the page asks *Choose Your Password*, the first step of a
wizard ([Responding to the user](web-application.md#responding-to-the-user)), a new password and its
confirmation, without the current one, and nothing else of the product
opens until it is chosen; every session of the account asks the same,
and Sign Out is the one other way out. A password chosen there is then
followed by the passkey offer, *Use a passkey next time?*, on a computer
as on a phone. An account that already chose its password and signs in by
a code is offered the passkey alone.

The offer names the way the device unlocks, from the browser's system:
*Sign in with Face ID or Touch ID instead of your password.* on an iPhone
or iPad, *Touch ID* on a Mac, *Windows Hello* on Windows, *your
fingerprint, face or screen lock* on Android, and *your device's screen
lock* elsewhere; its symbol is a passkey's key, the same on every device.

An account's other pages follow the choice: a page waiting on *Choose Your
Password* goes on by itself once another page of the account chose it.
A disabled account's sign-in says *This account is disabled. Ask your
administrator to enable it.*

Administrators manage accounts in Settings › Users, which only the master
and admins see, as macOS's Users & Groups or a GitHub organization's
People. For example, Ana, an admin, adds Bo for a new colleague: Add User…
asks his email and his role, and shows the first password the server made
for him, with Copy, beside a sign-in code; Bo's phone reads the code, or he
signs in with the password on his computer, and either way he chooses his
own password first. When Bo leaves, she disables his account.

- **The list** has one row per account: its nickname and email, its role,
  and when it was last active, which the account keeps after its sessions
  end, so a disabled account still says when it was last used, with *Disabled* on a disabled one and
  *Hasn't Chosen a Password* on one still to choose its own; the
  user's own row is marked *You*. A row has a ··· menu with what the user
  may do to that account, and none on the user's own or on one of a role
  not below theirs.
- **A first password** is always made by the server, 20 random letters and
  digits, never typed by an administrator: its owner replaces it at once
  anyway. Add User… and **Reset Password…** both end on it, with Copy, New
  Password, which makes another and stops the one shown, and a sign-in
  code beside it. A reset keeps the account's sessions and passkeys, and its
  owner chooses a new password at once, as above. The password is shown
  only there: an administrator who lost it makes a new one.
- **Sign-In Code…** shows a QR code that signs a phone in to that account,
  as a handoff code does: good for ten minutes and once, ending the
  account's earlier sign-in code, and asking the administrator's own
  confirmation as Hand Off to Phone does, Add User… and Reset Password…
  asking it the same way. Add User… shows it too once the account is made, so the new user's phone is signed in and adds a passkey
  there and then. The account's list of signed-in browsers names the
  administrator on a browser signed in this way, *Signed in with a code
  from Ana*. It grants no more than a password reset does, which already
  lets an administrator in, and changes no password.
- **Change Role…** moves an account between the roles below the
  administrator's own: the master makes users admins and admins users; an
  admin changes no role.
- **Sign Out Everywhere** ends every session of the account at once, its
  open pages included, as Sign Out beside a browser does for one.
- **Disable Account…** signs the account out everywhere and refuses its
  sign-ins, by password, passkey or handoff code, until **Enable Account**.
  Nothing of it is removed: its conversations, Clouds, devices and settings
  wait, a Cloud stops as a signed-out user's does, and its paired devices
  cannot reach the backend meanwhile. A disabled account is never deleted;
  removing an account and its data is not offered. Everyone can change their own password by providing the
current password, edit their nickname, and change email after verifying a code
sent to the new address. Setup and account creation do not require email delivery;
email change does, and reports an unavailable mail service explicitly.

An account can also sign in with a passkey, beside its password, which every
account keeps. For example, Ana adds a passkey in Settings › Account: she
enters her password, her browser offers to save the passkey in its password
manager, such as Google Password Manager in Chrome or iCloud Keychain in
Safari, and from then on any device that manager syncs to signs her in with
Touch ID or Face ID. The sign-in page offers her passkeys in the email
field's autofill and behind Sign In with Passkey, neither of which needs her
email first; a manager holding passkeys of several accounts lists each by
its email. An account has any number of passkeys, one per manager or
security key, and lists them in Settings › Account with the manager's name,
such as iCloud Keychain, whether it syncs (Synced) or stays on one device
(This Device Only), and when it was added and last used; the user renames
or removes one there, and changes the password there. Adding a passkey
always takes a confirmation that it is the user in the last five minutes,
the password or a passkey, a sign-in counting as one, so a browser left
signed in long ago cannot add one; instance setup offers the new master
account to add one right after its password, which it has just entered. An administrator's password reset
leaves the account's passkeys as they are.

Passkeys belong to the host name of the backend's public URL, such as
`demi.example`: a backend reached at another name, or by an IP address,
for which browsers make no passkeys, offers none.

Handoff carries what the user is doing from a signed-in browser to a phone,
by a code, as Handoff carries work between a Mac and an iPhone. For example,
Ana works in a conversation on her laptop and wants to follow it from her
phone: she presses the phone mark left of Settings at the sidebar's foot,
*Hand Off to Phone*, confirms it is her with her password or a passkey, and
a QR code appears. Her phone's camera reads it, the phone's browser opens
the conversation she was on, signed in, and the laptop's dialog says
*Handed off to Safari on iPhone.* and closes. On the phone, the page the
code opens says *Handing Off* while it opens the conversation, and *Couldn't
Hand Off* with the reason when the code has expired or was used. The phone mark stands only where the page is not itself on a
phone.

- **Confirming it is the user** takes the password, or a passkey when the
  account has one and the backend offers them, and holds for five minutes
  on that browser, a sign-in counting as one, so a second code in that time asks nothing; a browser
  left signed in cannot hand the account on without its owner.
- **The code** opens the backend's public URL with the page the laptop
  shows, so the phone lands where the user was. It is good for ten minutes
  and once; a new code ends the one before. An expired code's dialog offers
  Show New Code, which asks nothing while the confirmation holds.
- **A phone already signed in keeps its session.** A browser signed in to
  the account opens the page in the session it has, and no second session
  of it is made. A browser not signed in gets a session of its own, as a
  password sign-in on the phone would: signing out on one leaves the other
  signed in. A browser signed in to another account signs that one out and
  takes this one, so it holds one session, never two.
- **The phone can add a passkey at once.** An account still to choose its
  password chooses it first ([Administrators manage accounts](#user-system)).
  A sign-in by a code counts as the phone's confirmation, as a password sign-in does, so for five minutes
  the phone adds a passkey without asking the password again. The page the
  code opens offers it once on that browser, as a one-step wizard, *Use a passkey next time?* with
  Add Passkey and Not Now, and not again on that browser whichever is
  pressed; a phone whose password manager already holds a passkey of the
  account answers so, and the page says *This phone already has a passkey
  for Demi.* The passkey then signs the phone in by itself and, by the
  browser's own *Use a phone or tablet* code, signs in a computer too.

A computer signs in with a passkey on the phone without a code of Demi's:
Sign In with Passkey on the computer shows the browser's own choice of a
phone, whose QR code the phone's camera reads, and Face ID on the phone
signs the computer in. This is the browser's cross-device passkey sign-in
(both need Bluetooth on), which a passkey of any manager offers.

Settings › Account lists the browsers signed in to the account: each by its
browser and system, such as *Safari on iPhone*, or *Web Browser* when its
browser cannot be told, when it signed in and was
last active, the one the user is reading marked *This Browser*. Sign Out
beside another ends that session at once, its open pages included, so a
code photographed off the screen and used by someone else is undone there.

The web app's HTTP requests, the synchronization channel and conversation
WebSockets use the same session cookie.
[Backend authentication](../backend/backend.md#authentication-and-ownership)
owns session and lockout behavior; [Web API](web-api.md#account-api) owns
account request shapes and verification limits. There are no organizations or
team roles.
