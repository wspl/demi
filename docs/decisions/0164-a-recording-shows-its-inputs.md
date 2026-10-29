---
status: proposed
---

# A recording shows its inputs, and says which of them failed

## Problem

Alex asks an agent to accept a change on the running product with a video.
The agent drives a tab with `demi browser`, records it with
`demi browser record`, and hands Alex an MP4. In the video a chart's number
changes, a switch flips, a tab appears in the work panel and the settings
open, and nothing says why: the recording keeps the tab's pixels, and a
browser that Chrome drives draws no pointer. A hover, a click, a right
click, a wheel turn and a key press all look like the page changing by
itself. A message typed with `fill` appears whole. A simulator's recording
shows no tap and no swipe either.

The agent fares no better. It writes its takes as scripts of commands
between `sleep`s, starts a recording, runs the script and reads the frames
afterwards. In one acceptance video of five parts:

- One take ran 1 minute 42 seconds for nothing: `fill` was given its text
  the wrong way, failed, and the take went on without the message.
- One part was recorded four times. A wheel turn landed on an iframe, which
  took it, and then the scroll went the wrong way; each time the frames only
  showed that nothing moved.
- A click by coordinates missed its button, and the frames could not show
  where it landed.
- The agent lost a recording's ID, and found it by running `record rN stop`
  for N from 1 to 80, reading which tab each one named, which stopped every
  recording it reached.

## Decision

A recording keeps every input its source took while it ran, from the agent
or from the user's hand in the live view, and every command of the agent
with how it ended. An exported MP4 and an exported image draw the inputs
on the picture; `record rN frames` lists the commands among the frames; and
`record t1 list` names the recordings of a tab or a device.

### What a video shows

```text
            ╭───╮
   ↖  ◯     │ ⌘ │ ,      ↖ ⟳ ▼▼
   left     ╰───╯        wheel down, two notches
   click     a shortcut, at the bottom right
```

- **The pointer**, an arrowhead with no tail, black in a white outline over
  a soft shadow, moves to each place the source moved it. A jump, as an
  agent's moves are, eases there over 150 ms, so the eye follows it; it
  never arrives later than the input did. Before its first input it rests
  where that input will be.
- **A press** draws a ring that grows and fades from the point, 400 ms: blue
  for the left button, orange with a small `R` for the right one, grey with
  `M` for the middle one. One mouse makes one press at a time, so a press
  ends the ring of the one before: presses in quick succession never read
  as several fingers. The arrow takes the ring's color while the ring
  shows, and while a button is held, as in a drag, when it also shrinks
  and a line follows it.
- **A wheel turn** draws a chevron by the pointer in its direction, one for
  each notch of the turn, fading in 600 ms.
- **A key** that is not plain typing, Enter, Escape, Tab, an arrow, Delete
  or any combination with a modifier, draws its keys in a capsule in the
  bottom right corner, `⌘ ,`, `Shift ⇥`, which stays 1.5 s or until the next
  one replaces it. Text typed, inserted or filled draws a `⌨ 12` capsule
  there, the number of characters so far, which grows as the typing does:
  the text itself shows where it lands, and a password would show nowhere
  else. A character typed with Shift, AltGr or Option is typing, never a
  shortcut. Screen Studio and KeyCastr's shortcut mode draw the same.
- **On a simulator** a touch is a grey disc where the finger is, as iOS and
  Android draw touches for a demonstration; a swipe leaves its trail, a
  long press grows its disc, and a button, Home, Lock or a volume key, draws
  a capsule.

Sizes are in points, so the pointer is as large on a Retina capture as on
another.

`record rN export --mp4` draws them; `--plain` gives the video as it was
captured, which it is quicker to give, since only a drawn one is coded
again. A Host that cannot load the video coder gives the video plain and
says why, in the answer's `undrawn`. An exported image draws what was on the screen at its frame: the
pointer, and a press or a key that was still showing.

The live view draws nothing of this: the user sees the page, and their own
pointer.

### What the agent reads

`record rN frames` lists the agent's commands among the frames, at the time
each first gave the screen an input, or when it began for one that gave
none, with the element it acted on and how it ended. A command is listed by
the recordings that ran when it began, so one that ends after its
recording does is in none, and none is in a recording begun after it:

```text
FRAME  TIME (S)  CHANGED AREA OR INPUT
f001   0.000     whole
-      0.892     click --css "#q" → textbox "Search"
-      0.918     key Enter → textbox "Search"
-      0.965     click --text-match "Right click me" --button "right" → generic
f002   0.986     40,40 346x32
-      0.998     scroll --xy "400,300" --dy 500
f003   1.034     40,40 384x32
-      2.031     click --text-match "Nothing here" · failed: browser target matched no elements
-      2.540     (the user) right press at 220,96
-      3.100     (the user) typed 12 characters
```

A command is listed as the CLI takes it, its operands in their places and
then its flags, with the element it acted on. What it types is counted and
never quoted: the text of `type`, `fill`, `clipboard.write` and a prompt's
answer, and a `key` that types a character; a protocol message's or a
tool's arguments are kept by their size alone. The failure of such a
command is not kept either, since its message may quote them. Commands that
run at once on one tab or device each keep their own recordings and their
own first input. Only the
commands that act on the page or the device, or wait for it, are listed;
those that only read it, `inspect`, `find`, `screenshot`, are not. The
user's moves are not listed, and the keys they type in a row are one line.

A failed command says why, as the command said it, and a command that
changed nothing, as the scroll above, shows no frame after it. An input
from the user's hand is listed as what it was, since no command names it.
A command the CLI refused before it reached the browser, such as one with
a missing argument, never reached the recording either; its failure is in
the agent's own output.

`record t1 list` lists the recordings of the tab `t1`, newest first, and
`record m1 list` those of a device, open or closed: each one's ID, whether
it runs or why it ended, how long it ran and how many frames it kept.

### What is stored

A recording keeps one more file, `inputs.jsonl`, written as its
`frames.jsonl` is, one line an input or a command, in the frames' time and
pixels:

```rust
/// One line of `inputs.jsonl`: an input at its time, in microseconds from
/// the first frame.
struct InputRow { time_us: u64, input: Input }

enum Input {
    Pointer { x: f64, y: f64, scale: f64, phase: InputPhase, button: Option<InputButton>, by: InputBy },
    Wheel { x: f64, y: f64, scale: f64, dx: f64, dy: f64, by: InputBy },
    Keys { keys: Vec<String>, by: InputBy },
    Typed { characters: u32, by: InputBy },
    Touch { x: f64, y: f64, scale: f64, phase: InputPhase, by: InputBy },
    Button { name: String, by: InputBy },
    Command { text: String, failed: Option<String> },
}
```

`scale` is how many frame pixels a point is where the input was taken: the
recording's last frame over the tab's CSS width or the device's points,
long side to long side, which holds for a capture at any size, an Android
emulator's at its logical size on a Host with no hardware coder included;
before the first frame, the device pixel ratio or the screen's pixels stand
in for it. A recording an earlier release made has no `inputs.jsonl`,
and exports as it did.

A source reports an input where it hands it to the screen, so an agent's
input and the user's go the same way:

- A tab's: where the browser plugin sends Chrome a mouse, wheel, touch, key
  or text event, and where `fill` sets a field's value itself.
- A simulator's: the driver every input of the mobile plugin goes through,
  for iOS and Android alike.

An input's time is when its source handed it over, on the clock the
recording reads its frames' arrival by. A frame arrives the time its capture
and coding take after it was drawn, the first one often later than the
rest, so each frame also keeps when it arrived, and an input's time moves by
how much sooner than the first any frame arrived. An input then shows as
far before the frame that shows its effect as the page took to answer it.

### How exact it must be

A drawn export, and the times `frames` lists, are what the user and the
agent judge the product by, so a wrong one leads them to false conclusions.
Two bounds hold, measured on the running product:

- A press is drawn where the screen took it, to the pixel.
- A press is drawn before the frame that shows the screen's answer, by no
  more than 50 ms: never after it, which would show an effect before its
  cause.

### What an export costs

A drawn MP4 is the video decoded, drawn on and coded again with OpenH264,
which the export already loads to decode, in software: a hardware coder
works behind its calls and may drop a frame it has no room for, and an
export keeps every frame. It is coded at 30 frames a second while something
moves and holds the picture while nothing does.

## Spike

- **What a drawn export costs.** A 25.6-second recording at 1440 × 900,
  decoded and coded again at 30 frames a second throughout, took 2.8 s on
  a Mac with VideoToolbox and 5.7 s with OpenH264 in software. The export
  codes 30 frames a second only while something drawn moves, and holds
  the picture otherwise: the same recording, with its inputs, took 1.2 s.
- **How exact it is.** On a development server, a page drew a square
  where it took each press, and the agent pressed 16 times in tabs at
  device pixel ratios of 1 and 2. Every press was drawn on its square to the
  pixel, and 9 to 39 ms before the frame that showed the square, over
  three runs. Before each
  frame kept when it arrived, the first frame's slower arrival drew the
  presses up to 75 ms early. An iPhone 17 Pro Max simulator, at 3, drew
  each tap's disc on the point tapped; its time is when the tap was handed
  to the simulator, which no page there could measure.
- **Linux.** The Linux programs are static musl executables, which cannot
  load a library at run time: OpenH264, which the drawn export decodes and
  codes with, does not load there ("Dynamic loading not supported"), so a
  Linux Host, a Cloud included, gives the video plain and says why. The
  same holds of today's export of images, which decodes with it too; that
  is a defect of its own, outside this record.

## Prior art

- [Cap](https://github.com/CapSoftware/Cap), an open-source Screen Studio,
  records the pointer's moves and clicks into a file beside the video
  (`crates/project/src/cursor.rs`) and draws them, with a spring that
  smooths the moves and a ripple for a click, when it renders
  (`crates/rendering/src/cursor_interpolation.rs`,
  `layers/click_ripple.rs`, `layers/keyboard.rs`). Demi keeps the same
  split, inputs beside the video and drawn at export, and adds the button,
  the wheel and the touch, which Cap's clicks do not tell.
- Playwright's trace viewer marks where each action acted on its
  snapshots, and lists the actions with their outcomes beside them, as
  `frames` will.
- Screen Studio and KeyCastr draw shortcuts and leave plain typing out.

## Alternatives

- Drawing the pointer into the captured frames: every move would become a
  changed frame, and `frames`, which the agent reads to see what the page
  did, would list the pointer's moves instead.
- Showing touches through the simulators' own developer settings: Android's
  works only for Android, iOS has none for a simulator, and neither shows a
  mouse.
- Drawing the inputs in the work panel's player instead of the file: an MP4
  handed to the user plays elsewhere too.

## Open questions

None.
