---
status: proposed
---

# A settled screen is one that looks the same

## Problem

Alex asks the agent to check an app's welcome screen on an iPhone
simulator. Every `demi mobile screenshot` and `inspect` of that screen
takes 1.8 s and says the screen was still changing, yet two screenshots
taken 4 s apart are the same to the pixel.

A screenshot or an inspect first waits for a settled picture: no new frame
from the device for 300 ms, at most 1.5 s. The iOS simulator reports new
frames for some screens whose picture does not change, so those screens
never settle and every read waits the full 1.5 s. A screen with a spinner
or a progress bar never settles either, though only a small part of it
moves. `tap text=` waits for the same settled picture, so it waits too.

## Decision

A screen is settled when no frame comes for 300 ms, as now, or when frames
keep coming but two pictures of it 300 ms apart differ in at most 0.5% of
their pixels. A screenshot or an inspect of such a screen returns in about
half a second, and says nothing of a change.

- **Two pictures.** Once frames keep coming past the first 300 ms, Demi
  takes a picture, and another 300 ms after the first was taken. The
  second picture is the settled screen, so a screenshot returns it rather
  than taking a third. A pixel differs when one of
  its colors differs by more than a sixteenth of the range, so a
  simulator's slight shading changes do not count.
- **0.5%.** A spinner or a progress bar changes less than that; a page
  that slides or fades in changes most of the screen. It is Maestro's
  threshold for a static screen.
- **Still changing.** A screen whose pictures keep differing by more, as a
  video or a page in transition, is waited for up to 1.5 s, as now, and
  then taken as it is, with the note that it was still changing.

## Spike

Each screen was shot three times with today's build; the share of changed
pixels compares the first shot with the last, 1 to 4 s apart.

| Screen | Each shot | Changed |
| --- | --- | --- |
| iOS Settings, Maps, Photos, Calendar | 0.4 to 0.9 s | 0% |
| iOS News, Fitness, Health welcome screens | 1.7 to 2.0 s, still changing | 0% |
| A shopping site in iOS Safari | 1.7 to 1.9 s, still changing | 0.15% |
| Four other sites in iOS Safari | 0.5 to 0.8 s | 0% |
| Android Settings, launcher, five sites in Chrome | 0.4 to 1.0 s | 0% to 0.01% |
| An Android page with a spinner | 1.7 s, still changing | 0.09% |
| Android Calendar's first page as it opened | 1.6, then 0.6 s | 7.4% |

Four screens of 21 were still changing on every shot. Three of them did
not change at all, and the fourth changed in a strip of 146 by 23 pixels.

With this decision built, on a development build, each shot of those four
screens and of the spinner took 0.8 to 1.0 s, none still changing; each
shot of a still screen took 0.5 to 0.6 s, as before. Of the 0.9 s, 0.2 s
is comparing the two pictures unoptimized, which an optimized build does
in 2 ms. A screenshot taken during a page's transition still showed the
page once it was in place.

## Prior art

- [Maestro](https://github.com/mobile-dev-inc/maestro/blob/main/maestro-client/src/main/java/maestro/utils/ScreenshotUtils.kt)
  waits for an app to settle by reading the view hierarchy until two reads
  are the same, and its `waitForAnimationToEnd` until two screenshots
  differ by at most 0.5%. Demi keeps its frame signal, which costs nothing
  on a still screen, and compares pictures only when frames keep coming.
- Playwright waits for an element to be stable over two animation frames;
  it has no notion of the whole page settling.

## Alternatives

- **A shorter limit than 1.5 s.** Faster on a screen that never settles,
  and a screenshot of a page halfway through its transition on one that
  does.
- **Comparing view hierarchies, as Maestro does.** A read of the tree
  takes 0.3 to 0.6 s, so two of them cost more than the wait they save,
  and a tree does not show a picture's fade.
- **Comparing every frame.** The device's frames come many times a
  second; two pictures are enough to tell a still screen.
