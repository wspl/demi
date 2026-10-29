---
status: proposed
---

# A recording's video is made without other tools

## Problem

Alex asks an agent for an acceptance video of a change. Even with the
inputs drawn (0164), the agent makes it with tools of its own:

- It writes each part's caption with PIL into a transparent image and lays
  it over the video with ffmpeg, since the Host's ffmpeg has no text
  filter, and places it by eye, over the key it pressed in one take and
  over the page's own text in another.
- A recording runs two minutes at most, so the video is five recordings,
  which it joins with ffmpeg's concat.
- Twenty seconds of it are the page waiting for a model's answer, which
  Alex watches go by.
- To see a recording at all it samples frames with ffmpeg and tiles them
  with PIL, since a model reads no video. One such sheet covered 28
  seconds of a 37-second recording, and the agent concluded that its last
  steps had not run, which they had.

Each time the agent records again it rebuilds this pipeline from its own
earlier commands.

## Decision

`record` makes all four itself.

### Notes

```text
demi browser record r1 note "2/5 Hover the chart, then flip a switch"
demi browser record r1 note "3/5 Dark theme" --at 31.5
```

A note is text the agent sets at a time of a recording: now while it runs,
or at `--at` seconds of it, running or not, as when a take is captioned
after it is watched. It shows from its time until the next note, or the
end. `frames` lists each note at its time, among the commands and the
frames. A recording that ended takes a note only with `--at`, and one that
another program records takes it only from that program, which keeps its
files.

A drawn video shows the note as a video shows a subtitle: white text in a
box of translucent black, at the bottom left of the picture, two lines at
most and three fifths of its width. Its text is as large against the frame on every screen, a
forty-eighth of the frame's longer side, since a video is watched shrunk
to fit. A note longer than two lines is drawn smaller, then cut, and the
answer says which. The key capsule sits in the bottom right corner,
as far from the edges as the note, so the two share a row and never cover
each other.
`--plain` leaves notes out as it leaves the inputs out, and so do the
images of frames, which are for reading the screen's pixels; `frames`
lists the notes among them.

A note is drawn with the system's own font for the text, Hiragino Sans GB
on macOS, Microsoft YaHei on Windows, and Noto Sans CJK where a Linux Host
has it, and an emoji from the system's emoji font, in color where its
glyphs are pictures, as on macOS and in Noto Color Emoji. A character none
of them holds is drawn as a box, and the answer says which. Characters are
drawn one by one: a script whose letters join or run right to left, such
as Arabic, and an emoji made of several joined, are not drawn as written,
and the answer says so.

### One video of several recordings

```text
demi browser record r1,r2,r3 export --mp4 --output acceptance.mp4
```

`export` takes several recordings of the conversation, in the order the
list names them, as `--frames` takes several frames, from any tab or
device. The video plays them one after another, each with its own inputs
and notes. `--sheet` takes several the same way; every other action takes
one.

The video has one size, its first picture's, and a recording of another
size is scaled to fit it, in its middle on black, as a video editor fits a
clip to its project; a device rotated within a recording is fitted the same
way. An MP4 whose pictures change size names the first size as the whole
track's, which players stretch the others to, and a player keeps the
parameter sets of the first sample entry, which cannot decode another. So
a video of several recordings is coded again even with `--plain`, and a
Host that cannot load the video coder refuses it and says why.

### Waits played faster

`--fast-waits` plays every stretch of more than two seconds in which no
frame changed and no input came at eight times its speed, with a `▶▶ 8×`
badge at the top right of the picture while it does, as a player shows its
speed. Its first second plays as it was, so what
changed last is seen, and its last half second, so the next change is seen
coming. `frames` and every time an answer names stay in the recording's own
time; the answer says how long the video plays.

### A sheet of the whole recording

```text
demi browser record r1 export --sheet
demi browser record r1 export --sheet --every 1s
```

One PNG of the whole recording: a frame every `--every`, by default the
interval, a whole number of tenths of a second, that gives at most 24
tiles, laid out in rows, each labelled with the recording and its time and
drawn as a drawn export draws it, its note over it at a size that reads at
the tile's. `export --sheet | xargs demi file view` shows it. Every tile is
a time of the recording, the first at 0 and the last at its end, so a
sheet never leaves out the end of a recording. A sheet holds at most 120
tiles: its pixels are held whole while it is made, and more tiles shrink
past reading.

### What is stored

A note is one more kind of input, `Note { text }`, in `inputs.jsonl`, at
the time it was set or `--at`.

## Spike

None: notes, joining and the sheet use the drawn export of 0164. The build
checked on a Mac that the system's fonts draw Chinese, Japanese, Korean and
color emoji, and that joining recordings of different sizes into one MP4
of several sample entries plays stretched, which the one size above
avoids.

## Prior art

- Screen Studio and Loom caption a recording in their editor, below or
  over the picture; Demi's notes are set by the command that records.
- Screen Studio speeds up the stretches in which nothing moves, and marks
  them, as `--fast-waits` does.
- ffmpeg's concat demuxer joins videos of one size and one coding; Final
  Cut Pro and Premiere conform a clip of another size to the project's,
  fitting it by default, as an export of several recordings does.

## Alternatives

- A band of its own below the picture for the notes, which covers nothing
  of the screen: a full-width bar under every frame reads as part of the
  page, and changes the video's shape. A subtitle covers a little of the
  bottom of the screen, which is where viewers expect it.
- One recording longer than two minutes instead of joining: a recording's
  limit keeps a Host's disk safe, and an acceptance video is made of takes.

## Open questions

None.
