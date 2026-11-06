---
status: accepted
---

# A handoff code is also a link to copy

## Problem

Alex works on a laptop and wants the same account open on a desktop
computer that has no camera. Hand Off to Your Phone shows only a QR code,
so Alex signs in on the desktop with the password instead. An
administrator who makes a Sign-In Code for Sam, who works elsewhere, has
to photograph the code and send the picture.

The QR code is already a link: the backend's address with the code after
`#`, spent once within ten minutes. Only the dialog keeps it from being
copied.

## Decision

**While a code is valid, the dialog's footer offers Copy Link before
Done.** It puts the code's URL on the clipboard and reads Copied for a
moment, with the width of the wider label, as Share's Copy Link does. An
expired code shows Cancel and Show New Code, as today, and a spent one
shows Done.

```text
  Hand Off to Another Device                  x

              +--------------+
              |   QR  code   |
              +--------------+
   Scan with your phone's camera, or open the
           link on another device.
                 Expires in 9:41

                         [ Copy Link ]  [ Done ]
```

Every wording of the dialog has it: Hand Off, an administrator's Sign-In
Code, and the last step of Add User and Reset Password. The dialog still
never shows the URL's text, so a shared screen does not give the code
away.

**Hand off names a device, not a phone.** The action is Hand Off to
Another Device: the sidebar's button, its tip and the dialog's title. Its
mark is a computer and a phone. The line under the code says the link may
be opened anywhere, and an administrator's says it may be sent:

| Key | Text |
| --- | --- |
| `handoff.title`, `foot.handOff` | Hand Off to Another Device |
| `handoff.confirm` | Confirm it's you. Your other device then opens this page, signed in to this account. |
| `handoff.scan` | Scan with your phone's camera, or open the link on another device. |
| `handoff.codeConfirm` | Confirm it's you. A code then signs {name} in to their account on their device. |
| `handoff.codeScan` | Scan with their phone's camera, or send them the link. |

Add User's and Reset Password's last step says the same line under the code,
and the sentence above the password says "Give them this password, or sign
them in with the code below."

A page on a phone still has no Hand Off button.

The design to build from:

- `HandoffDialog.vue` copies `code.url` with `useCopy`; the backend and
  the protocol do not change.
- Every name that says phone for this action says device: the sidebar's
  `handOffToPhone` event becomes `handOff`, and the comments of
  `handoff.ts`, `HandoffDialog.vue` and the backend's `sessions.rs` say
  device.

## Prior art

Chromium's QR code bubble shows a page's link as a QR code with Copy and
Download beside it. Element signs in a new device by a QR code alone
(MSC4108), and Signal links a desktop the same way: both bind the code to a
camera so it cannot travel. Demi accepts the link because the code is
spent once and lasts ten minutes, and because its servers are run by the
people who use them.

## Alternatives

- Copy Link at the footer's leading edge, apart from Done: macOS puts an
  action that stands apart from the dialog's purpose there, and this one
  serves it.
- A small Copy Link under the expiry: the dialog grows taller and Done
  moves down.
- Showing the URL in a field: a shared screen or a screenshot would carry
  the code.

## Open questions

None.
