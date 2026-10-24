---
status: proposed
---

# Typed text lands in the field, or the command says what the field holds

## Problem

Alex asks the agent to sign in to an app in an Android emulator with a test
phone number and the fixed code 2496. The code field is four boxes, one
digit each, and each box moves the focus to the next after it takes its
digit. The agent runs `demi mobile type m3 2496`. The command prints "Typed
4 characters.", but the keys reached the app faster than the boxes moved
the focus: three digits fell on a full box and were dropped, or landed in
the wrong box. The app submitted a wrong code twice, the sign-in service
locked the emulator out, and Alex had the agent delete the emulator and
make another.

The new emulator, made with `avdmanager`, had no hardware keyboard
(`hw.keyboard=no`). Every key `type` and `key` sent went nowhere, and each
command still said it typed or pressed. Its first focused field opened a
keyboard tutorial over the whole screen. The agent typed the rest of the
session through `adb shell input keyevent`, one digit a call.

On an iPhone simulator whose keyboard is Pinyin, `type` sent `abc def` as
keys, and the field ended up holding `abc de f` in an unconfirmed
composition. When no field had the focus, `type` still said it typed.

In each case the command reports success it did not check. The agent
learns of the failure, if at all, from a screenshot or from the app.

## Decision

`type` checks that every character landed, and says what the field holds
afterwards. When it cannot make the text land, it fails and says why.

```text
$ demi mobile type m3 2496
Typed 4 characters into 4 fields: "2", "4", "9", "6".

$ demi mobile type m3 "abc def" --into e7
Typed 7 characters into text field "Search" [ref=e9]; the keys left "abc de f",
so it set the text directly; it holds "abc def".

$ demi mobile type m3 hello
demi mobile type: no field has the keyboard focus; tap one first, or
name it with --into
```

### How `type` types

1. Before the first key, a field must have the focus. With `--into`, `type`
   taps the field first. A screen whose tree names no focused field while
   the software keyboard shows, as a web page whose accessibility is still
   loading, has a field that takes keys: the text goes in unchecked, and
   `type` says so.
2. The first character goes alone. `type` reads the tree until the
   character shows in the field, then watches 500 ms from then for the
   focus to move. A field is known by its role, label and identifier, its
   place telling apart only fields alike in those, as a row's boxes; so a
   field that moves as a message above it goes is the same field.
   - If the focus moved, the field is one of a row that takes a character
     each, as a verification code's boxes. Every later character also goes
     alone: `type` waits until the focus stops moving, sends it, and checks
     the app took it: the box changed, the focus moved on, or the box is
     gone, as when the app submits a full code and clears its boxes. A
     character the app did not take, its box still focused and unchanged,
     is sent once more; if the app still does not take it, `type` fails and
     says which characters landed where. `type` says what each box read
     once it took its character.
   - If the focus stayed, the rest goes as one burst, as now, and `type`
     reads the field once at the end.
3. At the end, the field must hold its text from before with the typed
   text where the cursor was, in place of the text selected, if any. If it
   holds something else, `type` puts the text in through the platform's
   own set-text path (`--replace`'s), after the field's own text, as where
   the cursor was is lost once the keys went wrong, and says it did. A
   field that formats what it takes, such as a phone number with spaces,
   still holds its own form of the text then, with every typed character
   in order; `type` says what it holds. A field that lacks some of them,
   as one that takes at most four characters, makes `type` fail, saying
   what it holds. A field that is gone, as the app left the screen once
   it had the text, is not set: `type` says it is gone.
4. A password field's text cannot be read. `type` paces its keys 50 ms
   apart, as Maestro does, and says the field was not checked.
5. Enter and Tab act on the app: a submit, or a move of the focus. The text
   up to the first of them is checked as above; the rest goes in unchecked,
   and `type` says so.

An empty Android field reports its hint as its text. `inspect` reads it as
empty. `type` reads a field whose text is its hint as holding either
nothing or the hint's words, since the tree cannot tell the two apart.

While no field of a web page has the focus, Safari says every field has
it. A tree in which several fields say they have the focus names none.

A single field costs the 500 ms watch and a few tree reads more: 16
characters into an Android field took 1.1 s. A four-box code took 0.5 s on
Android when its boxes move the focus at once, 1.4 s when they wait 300 ms,
and 3.6 s on iOS, whose tree reads take about 0.6 s; each code was right.

### Android without a hardware keyboard

When an emulator has no hardware keyboard, `type` and `key` send their keys
through Android's own input injection (`input keyevent`, and `input
keycombination` for Shift), which needs none. `open` reads whether the
emulator has one from `dumpsys input`, as it opens it: an alphabetic
keyboard other than Android's `Virtual` one. No configuration of
the user's emulator changes.

### Keyboard tutorials

When `demi mobile open` opens a device, it turns off the first-run keyboard
tutorials that cover the screen: Gboard's stylus tutorial on Android
(`settings put secure stylus_handwriting_enabled 0`), and the swipe-to-type
introduction on iOS (`DidShowContinuousPathIntroduction` in the keyboard's
preferences). Both are settings of the simulator that a person turns off
once with a tap.

### Composing keyboards on iOS

A composing keyboard, such as Pinyin, turns typed keys into a composition.
Step 3 finds the field holding something else than the text and sets the
text directly, which also ends the composition.

## Spike

- **Fast typing into a code row.** A test app had a row whose boxes move
  the focus a frame after a digit, and a row that moves it after 300 ms,
  as an animation does. On Android, `type 2496` filled the first row five
  times out of five and the second none out of three: one digit landed,
  three were dropped. On iOS, the first row took the code four times out
  of seventeen, and once took `2466`, a wrong code the page submitted.
  One digit a call, 0.3 s apart, took it three times out of three.
- **Typing one character at a time with a check.** A script sent each digit
  with `input keyevent`, read the tree until it landed, and sent it again
  if it did not. It filled the 300 ms row correctly in 3.9 s, with the
  command line's own start-up in each read. Inside `demi-mobile` a read
  takes about 0.3 s on Android and 0.6 s on iOS.
- **No hardware keyboard.** On an emulator with `hw.keyboard=no`, `type`
  (keys and paste) and `key delete` changed nothing and reported success;
  `type --replace` and `adb shell input keyevent` worked. An emulator with
  the keyboard on took every key. `dumpsys input` tells them apart: with
  the keyboard on, it lists an alphabetic keyboard (`qwerty2`) besides
  Android's own `Virtual` one; without it, only `Virtual`.
- **Tutorials.** A new emulator without a hardware keyboard showed Gboard's
  stylus tutorial over the whole screen at the first focused field, and
  the tree held only a keyboard. With `stylus_handwriting_enabled` set to
  0 first, it showed none. A new iOS simulator showed the swipe-to-type
  introduction under the field.
- **Pinyin.** Typing `ab` left `a b` in a composition; `abc def` left `abc
  de f`. Setting the value through accessibility put `ab` in plainly and
  ran the app's search for it, as typing does.
- **No focus.** On iOS, `type` with no focused field said it typed 17
  characters.

## Prior art

- [Maestro](https://github.com/mobile-dev-inc/Maestro/pull/3334) types one
  character an event on Android, because fields split into one box per
  character need each event to finish its focus change before the next.
  It does not check what landed. Demi checks, and keeps one burst for a
  single field, which Maestro does not.
- [Appium's XCUITest driver](https://appium.github.io/appium-xcuitest-driver/latest/reference/settings/)
  limits typing to `maxTypingFrequency`, 60 keystrokes a minute by
  default, because faster typing drops characters. A fixed pace is slow
  for long text and still loses to a slow app; Demi waits for each
  character instead.
- [appium-ios-simulator](https://github.com/appium/appium-ios-simulator/pull/315)
  sets `DidShowContinuousPathIntroduction` when it boots a simulator, so
  the keyboard tutorial does not block typing. Demi does the same at
  `open`.
- Appium's UiAutomator2 driver sets text through accessibility by default,
  not with keys. Demi keeps keys first, because an app's key handling,
  such as a code row's focus moves, only runs for keys.

## Alternatives

- **A fixed pace for every key.** Simple, but a 100-character message at
  Appium's pace takes over a minute, and no fixed pace is slow enough for
  every app.
- **Always one character at a time with a check.** Correct, but a long
  message costs a tree read per character, about 30 s for 100 characters
  on Android.
- **Turning the hardware keyboard on in the emulator's `config.ini` when
  `open` boots it.** It hides the on-screen keyboard and its tutorial, but
  changes the user's emulator file, and does nothing for an emulator
  already running without one.
- **Switching the iOS keyboard to English.** Appium's `keyboard` setting
  does this, but it changes the user's simulator for everything they type
  in it.
- **Reporting a keyboard tutorial instead of turning it off.** The agent
  would still have to find the tutorial's button and tap it, on every new
  simulator, for a setting a person turns off once without a thought.
