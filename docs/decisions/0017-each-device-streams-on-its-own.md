---
status: accepted
---

# Each device streams on its own

## Context and Problem Statement

A conversation's simulators share one live stream, which carries one device
at a time: showing another device's tab tells that stream to watch another
device. "The device being watched" is therefore one value for the whole
conversation, and it is wrong whenever the page and the Host disagree about
it, which a network delay makes them do at every quick switch:

* The Host's late answers overwrite the page's newer choice, and the page
  answers each with another switch. A few tab switches send dozens of them.
* A late picture of the device left a moment ago paints into the new tab,
  and the tab's frame resizes to it.
* A device left on screen can count as watched by nobody, and close as idle.
* Two devices can never be looked at together.

Each device's look is also assembled from parts that apply one by one: its
frame, the screen's shape, its orientation and its picture. The screen can
show before its frame or the other way round, and the frame kept in the
page's cache waits for the Host before it shows.

## Considered Options

* Keep one stream for the conversation's devices and fix each path.
* Give each device a stream of its own.

## Decision Outcome

Chosen option: a stream of its own for each device, because nothing then
switches, and a stream that carries one device cannot show another.

```
tab m1 -- stream m1 -- capture m1    looked at: pictures flow
tab m2 -- stream m2 -- capture m2    not looked at: open, nothing sent
tab m3 -- stream m3 -- capture m3
```

* **A stream per device.** Each open device has its own live stream and
  decoder, kept by the plugin's store while the device is open. The stream
  has one state of its own: whether it is being looked at, that is, whether
  a view of it shows in a visible page. While it is, the Host sends its
  pictures; while it is not, the stream stays open and the Host sends
  nothing, so a return shows the device at once and goes on with its next
  picture. No value anywhere says which device is wanted or selected.
* **Any number at once.** Several devices can be looked at together, in two
  pages or side by side, each streaming on its own. A device that is looked
  at never counts as idle.
* **A device's look changes whole.** Its frame, the screen's shape, its
  orientation and its picture change together, and a view shows them only
  together, so the screen and its frame always appear at once. Until the
  first picture arrives, the frame and picture kept from before show, from
  the page's cache, without waiting for the Host. A view that waits says
  what it waits for, and what failed.
* **Rotation** turns what the view shows at once, from the orientation of
  what it shows (decision record 0007), also after a reconnect.
* **The Host** captures each device on its own, so one device's work never
  holds up another's. Every device close logs why, with the device and when
  it was last looked at or used.
* **The screen's shape** comes from the device's own geometry, not from the
  transparency of the frame's artwork.

### Consequences

* Good, because no order of pictures or answers can show one device in
  another's tab, move its frame or leave a device on screen unwatched.
* Good, because looking at two devices together needs nothing more.
* Bad, because each open device holds a stream of its own while it is open,
  idle while nobody looks at it.
