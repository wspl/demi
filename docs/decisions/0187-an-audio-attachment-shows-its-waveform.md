---
status: proposed
---

# An audio attachment shows its waveform

## Problem

Alex asks for a voiceover, and the reply attaches `voice.mp3`. Its player
shows a play button and the browser's own range control as its seek bar: a
gray groove with a large round thumb in the accent color, drawn differently in
each browser. It says nothing of the sound itself, so two attachments, a
voiceover and a piece of music, look the same, and the thumb, the heaviest
thing in the row, is the part Alex looks at least.

## Decision

The seek bar becomes the file's waveform, drawn as a row of hairlines, as a
voice message shows in Messages and an editor shows an audio clip.

```text
 ( > )  voice.mp3                                    0:04
        ·|||l|ll|·····||l|ll||l|l··||ll|l|||····|l|l|·
```

- The player keeps its card, its 16 px corners and its name and time row. The
  play button stays round and grows to 36 px, the height of the two rows
  beside it, so the card's padding is 6 px above, below and left of it.
- The waveform is 80 hairlines of one width, evenly spaced, each as tall as the
  loudness of its stretch of the file: its root mean square, smoothed with its
  neighbors and scaled so the loudest is full height. Until the file is read,
  and for a file the browser cannot decode, every hairline is short, so the
  player never changes size.
- The part played fills with the accent; the rest is the text color at 20%.
- The browser's range control stays over the waveform, unseen, so a press or a
  drag seeks there and the keys and a screen reader work as before. Its focus
  shows as a ring around the waveform.
- `waveformOf(src, count)` in `web-ui/src/files/waveform.ts` reads the file
  once with `AudioContext.decodeAudioData` and keeps the result for every
  player of the same file on the page.

## Spike

None: the decoding is the browser's, and the gallery's voiceover and the
site's demo play and seek with it.

## Prior art

- [wavesurfer.js](https://github.com/katspaugh/wavesurfer.js) decodes the file
  with the Web Audio API and draws bars from each stretch's peak. This player
  draws the same way with the root mean square, which reads as loudness rather
  than as the loudest sample, and keeps the browser's range control for input.
- Messages and WhatsApp show a voice message as bars that fill as it plays.

## Alternatives

- A thinner styled seek bar: it fixes the thumb but still says nothing of the
  sound.
- Waveforms the server computes and stores: they would serve long files
  without reading them in the page, at the cost of a new stored value for every
  audio file; the page reads only the files it shows.

## Open questions

None.
