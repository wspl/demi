---
status: proposed
---

# A card asks to turn on notifications when none would reach the user

## Problem

Alex asks the agent to say when a long build ends, and walks away. The
agent runs `demi notify status` and reads a warning: notifications are off
in every browser of Alex's. The agent can only write that in its reply,
which Alex reads after coming back, too late. Alex never turned
notifications on, or turned them on in one browser that was since cleared,
and nothing on the screen says so.

## Decision

When `demi notify status` or `demi notify send` finds that no browser of
the user takes the agent's notifications, the conversation shows a card
above the composer, where a permission request shows:

```text
+------------------------------------------------------------+
| (bell) The agent can't notify you                          |
|        None of your browsers has notifications on. Turn    |
|        them on to hear when the agent finishes or needs    |
|        you, even with Demi closed.                         |
|                          [ Dismiss ]  [ Turn On Notifications ] |
+------------------------------------------------------------+
```

* **Turn On Notifications** turns on notifications in the browser the user
  clicks it in, and the agent's notifications with them, as the switches of
  Settings › Notifications do: the browser asks for its permission first
  when it has not been given.
  * In Safari on an iPhone outside the Home Screen, it opens the sheet that
    shows how to add Demi there, as the settings do.
  * In a browser that blocks Demi's notifications, the card says they are
    blocked in the browser's site settings and has no Turn On button, since
    the browser no longer asks. A browser that cannot notify at all says so
    the same way.
* Once any browser of the user's takes the agent's notifications, the card
  leaves every conversation that shows it.
* **Dismiss** removes the card from this conversation, and no command of
  this conversation shows it again. Other conversations still can.
* A conversation shows one card at most, however often its agent checks.
* A permission request, which holds a command, takes the place first; the
  card shows once no request waits.
* The card is not raised when the notification was held back only because
  the user has the conversation open: `notify send` says so to the agent
  already, and nothing is off.
* `notify status` and `notify send` tell the agent, in their descriptions and
  their warning, that the conversation shows the user a card to turn
  notifications on, so it need not explain how.

The design:

* The conversation stores the card's status in one column,
  `notifications_card`: none, `shown` or `dismissed`.
  * `notify status` or `notify send` sets `shown` when it finds no browser
    and the status is none.
  * Saving a subscription that takes the agent's notifications sets every
    `shown` of the user back to none.
  * Dismiss sets `dismissed`, which stays.
  * Each of the first two reads the subscriptions in the statement that
    writes the status, so a check that raced a subscription neither shows
    the card to a user it reaches nor clears it for one it does not.
* The conversation's metadata carries `notificationsCard: boolean`, true
  while the status is `shown`. A page that does not know it shows nothing.
* Dismiss is `POST /conversations/:id/notifications-card/dismiss`.
* Turn On Notifications changes the browser's own notification settings,
  the ones Settings › Notifications shows, and subscribes as they do; the
  backend clears the card when the subscription is saved.

## Spike

None.

## Prior art

* Zulip's web app shows a banner, "Zulip needs your permission to enable
  desktop notifications", with Enable notifications and Never ask on this
  computer, and hides it once permission is granted or denied
  ([`navbar_alerts.ts`](https://github.com/zulip/zulip/blob/main/web/src/navbar_alerts.ts)).
  Demi asks only when an agent finds the user unreachable, in the
  conversation that wanted to notify, and its Dismiss holds for that
  conversation, since another may matter more.
* Slack shows a banner at the top of the window until notifications are
  turned on or the banner is closed.

## Alternatives

* A banner at the top of every page until notifications are on: it asks
  users who never wanted notifications, and keeps asking.
* A dialog when the command runs: it interrupts whatever the user is doing,
  for an event the user did not start.
* A fourth kind of permission request: a request holds the agent's command
  until the user answers and notifies the user of itself, while this card
  holds nothing and cannot notify.
* Dismiss for every conversation: one dismissal would silence the card in a
  conversation where notifications matter.

## Open questions

None.
