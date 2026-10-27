---
status: proposed
---

# A device stays quiet while Demi is in front or another is in use

## Problem

Alex works at a laptop with Demi open on another conversation, and a turn
of the conversation *Draft the release notes* finishes. The laptop shows a
notification, and so does Alex's phone on the desk, because a browser is
held back today only from the conversation that browser itself shows,
focused and visible. Every turn that ends while Alex works at the laptop
buzzes the phone too.

The laptop is no better: Alex, reading one conversation, gets a system
notification for every other one that finishes, though the sidebar in
front of them marks it already.

Holding the phone back while a Demi page is merely open does not work
either: a page left in front of an empty chair looks the same as one in
use, and Alex, gone to lunch, would miss everything.

## Decision

A session is **in use** while one of its pages had the user's input, a key,
a click, a scroll or a pointer's move, in the last 10 minutes, and that page
is still connected. An open page alone is not in use.

A browser one of whose Demi pages is **in front** of the user, focused and
visible, is not notified of a turn that finishes or fails, in any
conversation: the page shows it, with the sidebar's marks. Today only the
conversation the page shows is held back.

A browser whose user turned on **Quiet while I'm on another device**, on by
default, is not notified of a turn that finishes or fails while another
session of the account is in use and its own is not. The session in use
notifies, as today, so the user at that device learns of it there; nothing
is sent to the quiet browser later.

* Once the other device has had no input for 10 minutes, or its pages
  closed or lost their connection, the quiet browser is notified again of
  each turn that ends from then on.
* A request that needs the user, the `Permission` notification, and the
  agent's own, `demi notify send`, reach every browser that chose them at
  once, as today.
* The rule is the same in both directions: while the phone is in use, the
  laptop stays quiet.

```text
            laptop in use              laptop idle for 10 min
  -------+---------------------------+-------------------------->
         |turn ends                  |      turn ends
         v                           |         v
  laptop: notifies, unless Demi      |  laptop: notifies, unless
          is in front                |          Demi is in front
  phone:  quiet, nothing later       |  phone:  notifies
```

Settings › Notifications, under Browser notifications, gains one switch,
disabled while notifications are off:

```text
  Browser notifications                                  [on]
  On this browser only. They come even with Demi closed.
  Quiet while I'm on another device                      [on]
  Turns that finish or fail don't notify here while you're using
  Demi on another device, until you've been away from it for 10
  minutes. Permission requests and the agent's notifications
  always come.
```

### Design

* The page says it was used, on its synchronization channel, at its first
  input and then at most once a minute while input goes on, and only while
  it is visible:

  ```rust
  pub enum PageMessage {
      // ...
      /// The user acted on the page, a key, a click, a scroll or a
      /// pointer's move, while it was visible.
      Used,
  }
  ```

* The page's `Showing` message says whether it is in front, `front`,
  beside the conversation it shows then; a page that does not say it is
  not in front.
* The backend notes when each page's channel last said `Used`, by its own
  clock, beside what the page shows, and forgets it when the channel ends.
  In use lasts `IN_USE_MINUTES`, 10 minutes, the time the user chose after
  Slack's and Discord's away timeouts.
* A subscription says whether it stays quiet, beside the events it chose:
  `PushSubscriptionBody.quiet_while_elsewhere: bool`, stored with the
  subscription. The setting belongs to the browser, as the others do.
* Whether a browser is left out as the user is on another device is
  decided as the turn ends, from what the pages said until then, however
  long the notification waits for the ones before it; nothing is kept for
  later.
* `demi notify send` tells the agent apart a user who has the conversation
  open, whose browsers it does not notify, from one whose browsers take no
  notification.
* `notification.sent` and `notification.unsent` log `quiet_elsewhere`, the
  subscriptions left out as the user was on another device, beside
  `suppressed_by_page`.

An older page never says `Used`, so its session is never in use and keeps
no other browser quiet, as today. An older subscription stored without the
field stays quiet, the default.

## Spike

None: the signal is the page's own input events.

## Prior art

* **Mattermost** sets a user away after `UserStatusAwayTimeout`, 300
  seconds by default, without activity, and its `push_status` setting
  chooses whether a phone notifies while the user is online, away or
  offline
  ([`doesStatusAllowPushNotification`](https://github.com/mattermost/mattermost/blob/master/server/channels/app/notification_push.go)).
  What came while the user was online is not pushed later. Demi does the
  same.
* **Discord** sends no mobile push while the desktop client is in use,
  until its *Push Notification Inactive Timeout*; its users asked for a way
  to always get them, which the switch here is.
* **Slack** considers the desktop app away after 10 minutes without input.
* **Zulip** sends a mobile push only to a user with no open client, and
  pushes what a closed client missed once its last event queue goes
  offline
  ([`missedmessage_hook`](https://github.com/zulip/zulip/blob/main/zerver/tornado/event_queue.py)).
  Demi sends nothing later: the device in use notified.
* The closed apps read the system's idle time; a web page sees only input
  to itself, so a user working in another app on the laptop counts as away
  after 10 minutes, and the phone notifies again.

## Alternatives

* **An open, visible page keeps the others quiet.** A page left in front of
  an empty chair would silence the phone for good.
* **Send what the phone missed once the user leaves**, as Zulip does. The
  user was at the device that notified; a second notification later on the
  phone is the noise this record removes.
* **The system's idle time.** The Idle Detection API runs only in Chrome
  and Edge, behind a permission prompt, and nothing tells which browser
  runs on the computer a runner could read it on.

## Open questions

None.
