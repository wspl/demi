---
status: proposed
---

# A transcript glides to the end it follows

Supersedes: 0051 (A transcript moves only for new content, and smoothly), the
part where rows slide when the list scrolls to follow its end.

## Problem

A reader on an iPhone watches a reply stream in. Each time the reply grows by
a line, the whole transcript drops by that line for one frame, then comes
back and slides up. At a line every quarter second, the text shakes. It
shakes more after the reader touches the list.

The list follows its end in two steps that must land in the same frame: it
moves the scroll position down by the growth, and it starts an animation that
draws every row from where it stood, so the rows slide up instead of jumping.
Chrome paints both in one frame. iOS Safari paints the animation a frame
before the scroll position, so for that frame the rows stand one line lower.

## Decision

The reader sees the text rise smoothly as it streams, on every browser: each
new line scrolls into view over about 150 ms, and no row ever moves back.

While the list follows its end, the scroll position itself glides to the end
a little in each frame, and no row is animated:

- **The glide.** Each frame, the view scrolls by part of what is left to the
  end, the part that shrinks the rest by e every `GLIDE_MS` = 50 ms, at least
  a pixel. It reads the end again in each frame, so text that keeps arriving
  only moves the target.
- **What starts it.** The list growing at its end while it follows, the
  reader's arrow to the end, a message sent, and the view catching up after a
  click held it.
- **Rows that reach into the view.** While following, a row that reaches into
  the view and grows at its end (the streaming reply) does not make the
  virtualizer scroll by its growth: the glide brings in what it added. What
  such a row grew above the text it ends with, as a formula drawn late does,
  is made up at once, so the text the reader reads stays still. A row wholly
  above the view still corrects the position at once.
- **The glide is the list's own scroll.** While it runs, every scroll is the
  list's, the glide's or its corrections': none ends following, and none
  moves a row, as the rows' painted places go with the glide.
- **The reader.** The reader takes the list by a finger on it, a wheel, a
  scrolling key, or a mouse or pen held on it or on its scrollbar: the glide
  stops, and what scrolls next is theirs. The finger's lift is heard by the
  window, as the row it touched can leave the list as the list scrolls. When
  the reader lets go and the list still follows, the glide resumes.
- **Going to the end ends a hold.** The arrow or a send right after the reader
  opened a row ends the view's hold for that row, so the list goes to the end.
- **Jumps stay jumps.**
  - An end further than a view away is jumped to.
  - A list not yet painted where it was placed, as a conversation opens,
    jumps, so opening shows no glide.
  - With reduced motion, or a hidden page, the list jumps.
  - The dock growing over the list, or the list getting shorter for an
    on-screen keyboard, keeps the end in the same frame (`keepEnd`), so the
    latest line is never hidden under the dock for a frame.
- **Row moves.** Rows still slide when one comes or goes while the list does
  not move to follow, as 0051 says.

```ts
/** How quickly the view glides to the end it follows (`useBlockVirtualizer`). */
const GLIDE_MS = 50

interface BlockVirtualizer {
  /** The reader goes to the end: following from there, gliding to it. */
  scrollToBottom(): void
  /** What covers the end grew, or the list got shorter: at the end at once. */
  keepEnd(): void
}
```

Nothing is stored and no protocol changes.

## Spike

An iPhone 17 simulator on iOS 26.5, in Safari, recorded at 60 frames a
second while a long reply streamed. Each frame was compared with the one
before to find how far the text moved.

| Build | Frames that drop by a line, then come back |
| --- | --- |
| Before | 15 in 40 s |
| Rows not animated while following, for a check | 0; each line jumps up |
| This decision | 0; each line rises over about 150 ms |

Chrome at 1280 × 800: each line rises over about 150 ms, and no frame moves
back.

## Prior art

- [use-stick-to-bottom](https://github.com/stackblitz-labs/use-stick-to-bottom),
  which AI chat apps use for a streaming transcript, animates the scroll
  position itself with a spring toward the bottom, read again as content
  grows, and tells its own scroll events from the reader's. Demi's glide is
  the same idea with an exponential approach instead of a spring.
- t3code jumps to the end as content grows, with no animation.

## Alternatives

- **Jump, with no animation, as t3code does.** Simplest, but the text moves
  in steps on every platform, where it slides today.
- **Jump only in Safari.** Two behaviors to keep apart, and Safari readers get
  the steps.
- **Native smooth scrolling (`behavior: 'smooth'`).** Its duration is the
  browser's, it restarts on each new target, and its scroll events cannot be
  told from the reader's.

## Open questions

None.
