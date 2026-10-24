---
status: proposed
---

# An element is found by its text or id, once it is there

## Problem

Alex asks the agent to delete a test account in an app on an Android
emulator. The agent taps Settings, sleeps half a second, takes a
screenshot, reads the coordinates of Delete Account from it, and taps
them. The settings page slides in slower than that once, and the tap lands
on the page before it, on a button that was under that point.

The agent works this way because a ref, the one way to name an element,
costs an `inspect` first and is good only until the next one renumbers
every element. In one long session, the agent tapped coordinates 251
times and a ref 19 times. It ran `sleep` 249 times and
`wait` never.

## Decision

Wherever a command takes an element, it also takes the element by what a
person reads on it, `text=`, or by its identifier, `id=`. The command
finds it in the tree when it runs, and waits until it is there.

```text
$ demi mobile tap m1 "text=Delete Account"
Tapped button "Delete Account" [ref=e41] at 352,1210.

$ demi mobile type m1 alex@example.com --into id=email
Typed 16 characters into text field "Email" [ref=e44]; it holds "alex@example.com".

$ demi mobile tap m1 text=Delete
demi mobile tap: 2 elements show "Delete": button "Delete" [ref=e45] at 120,980,
text "Delete" [ref=e46] at 352,300; tap one by its ref
```

- **Where.** `tap` and `swipe` take a target in place of a ref; `type
  --into`, `wait --element` and `wait --gone` take one too. A target is a
  ref from the last `inspect`, as now, `text=<text>` or `id=<identifier>`.
- **What matches.** `text=` matches an element whose label or value is the
  text, spaces at either end and runs of spaces aside. When none is, it
  matches one whose label or value holds the text, case aside. `id=`
  matches the identifier `inspect` shows, exactly. The keyboard's keys
  never match.
- **Once it is there.** The command reads the tree until exactly one
  element matches, its centre is on the screen, and it is not disabled.
  Then it waits for the picture to settle, as `screenshot` does, and reads
  the tree again: the element must still be at the same place. A page that
  slides in, or a window that opens, takes no input along its way, and the
  tree can say where an element ends up before it is there. Then the
  command acts on the centre of that element.
  It waits 10 seconds by default; `--timeout` changes that. A timeout says
  what it found: nothing, an element off the screen, a disabled one, or
  one still moving.
- **Several match.** The command fails at once and lists each match with
  a ref and where it is, so the agent taps the one it means. It does not
  guess.
- **Its ref.** The element found gets a ref, as `wait` gives one, so a
  later command can name it again.
- **`wait`.** `wait --element` with `text=` or `id=` waits as `tap` does,
  for the one match, on the screen, enabled and settled, with `wait`'s
  30 seconds. `wait --gone` waits until no element matches. `wait --text`
  matches as `text=` does, and waits for any match.
- **Timeouts.** The wait for a settled picture ends with `--timeout`
  too. A timeout says whether the element moved between two reads or the
  screen around it kept changing.

The help of `tap` says that it returns once the device took the tap, and
that `wait` waits for what the tap changes. The help of `tap` and `type`
says that `text=` and `id=` name an element without an `inspect`.

## Spike

- **Blind taps.** A test app opens a second page 1.2 s after a tap, over a
  button of the first page. A tap at that button's point 0.5 s after the
  first tap hit the first page's button. `wait --text` for the second
  page's button, then a tap, hit the second page's.
- **Refs.** Each `inspect` gives every element a new number, as e320,
  e340, e360 for the same button in three reads.
- **Reads.** A read of the tree takes about 0.3 s on Android and 0.6 s on
  iOS.
- **Stable over two reads alone.** With a tap on the second page's button
  as soon as two reads found it at one place, one tap in three reached
  nothing, as the window was still opening, and one tapped 73 pixels off
  the button's centre, where the tree placed it mid-animation. Waiting for
  the picture to settle between the reads made five taps in five hit the
  button, 2.7 s after the first tap, the page's 1.2 s delay included. A
  tap on a still screen takes about 0.9 s.

## Prior art

- [Maestro](https://docs.maestro.dev/reference/selectors/core-selectors)
  taps by `text` (a full match of a regular expression) or `id`, and polls
  for the element for up to 17 seconds. When several match, it takes one,
  or the one `index` picks. Demi lists them instead, since the agent
  cannot see which one Maestro would take.
- [Playwright](https://playwright.dev/docs/actionability) locators wait
  until the element is visible, stable over two animation frames, enabled
  and the only match, and fail on several matches in strict mode. Demi
  checks the same, with two reads of the tree for two frames. Its
  `text=` and `id=` follow Playwright's older selector engines of the
  same names.
- Appium finds by accessibility id with an implicit wait, 0 seconds by
  default.

## Alternatives

- **`--text` and `--id` options.** They fit `tap`, but `swipe` takes a
  direction after its element, and `type` names its element with `--into`
  already; one form of target fits every place an element goes.
- **Taking the first of several matches.** Quick when it is the right
  one, and a tap on a title instead of its button otherwise, which the
  agent learns of only from the screen.
- **Refs that last across `inspect`.** A ref names what an `inspect`
  listed, at its place then; keeping the number does not tell where the
  element is now, which is what a tap needs.
- **Waiting 30 seconds, as `wait` does.** A misspelled text then costs
  half a minute; an app that loads longer is waited for with `wait`.
