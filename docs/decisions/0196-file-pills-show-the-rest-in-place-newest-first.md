---
status: accepted
---

# File pills show the rest in place, newest first

Supersedes: 0188 (A binary file's pill opens the file), the part where the
"+N Files" pill lists its files in the Change view.

## Problem

The agent edits twelve files in one run. Under the folded row, two lines
of pills name the first files it touched, `README.md`, `package.json` and
so on, and a last pill says "+6 Files". The file it just changed, the one
Alex wants to look at, is among the six. A click on "+6 Files" opens the
Change view on one of them, so Alex leaves the conversation to find out
which files they are.

## Decision

**"+N Files" shows the files it counts where they are: a click puts every
pill on the lines below the first two, and the button becomes Show Less,
which folds them back. The pills stand newest first, so the files the
agent changed last are the ones that show.**

```text
  README.md +3 −1   app.ts +40 −2   util.ts +5   +6 Files
                    ─ click ─▶
  README.md +3 −1   app.ts +40 −2   util.ts +5   …
  config.json +1   index.html +12 −4   …   Show Less
```

- The button is a control wherever the pills show, whether or not a plugin
  opens changes, and says whether it is expanded to assistive technology.
  Show Less shows only while the pills need more than two lines.
- A call's pills stand in the reverse of the order its job first wrote
  each file; its removals and renames, which it lists after them, come
  first. A run's folded pills stand by the latest call that changed each
  file, newest first, and within a call as that call's own pills do. Which
  call that was, a rename of its folder or its removal included, is read
  from the run's list each call stored, the latest whose entry for the
  file differs from the one before it.
- A pill still opens its file, and every other control stays.

### Design

- `web-ui` `FileChangePills.vue` holds whether it is expanded and finds
  each pill by its path; the `restSelectable` property and the `openRest`
  event go, and with them `requestFileSelection`. `useFitMore` fits again
  only when its box's width changes, so expanding, which only makes the
  box taller, keeps the button and its focus.
- `files/request-changes.ts`: `callPills` answers a call's files newest
  first, and `runPills` a run's, ordered by the calls they come from as
  above, from the lists the calls stored. Nothing stored changes.

## Prior art

- **GitHub**'s comment threads fold hidden items into "N hidden items, Load
  more…" and show them in place.
- **macOS**'s Finder sidebar shows a section's rest in place with Show More
  and Show Less.
- A row's pictures in Demi already stand newest first, the oldest counted.

## Alternatives

- **Keep opening the Change view.** It shows one file of the rest, not
  which files the rest are.
- **Order the stored lists newest first.** Stored transcripts would keep
  the old order until a migration made every list again.

## Open questions

None.
