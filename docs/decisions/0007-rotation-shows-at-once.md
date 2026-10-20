---
status: accepted
---

# Rotating a device shows at once

## Context and Problem Statement

Rotate Left and Rotate Right in the device panel change nothing on screen
until the device has turned and sent its first picture in the new
orientation. Through a distant server that takes about half a second, during
which the panel looks as if the press did nothing.

This record covers rotation only. The other controls keep their behavior.

## Considered Options

* The panel turns the shown picture and its frame at once, and goes back if
  the device refuses.
* Show a busy state on the button until the device answers.
* Keep waiting for the device, as now.

## Decision Outcome

Chosen option: turn at once and go back on refusal, because the result of a
rotation is known before the device answers.

* A press turns the shown picture and its frame by a quarter at once.
* The device's pictures in the new orientation replace the turned one when
  they arrive.
* If the device refuses, the picture and frame turn back to the device's
  orientation, and the error shows as it does now.
* Several quick presses add up, and the panel ends in the orientation the
  device ends in.

```text
  press ⟳                 at once                  device answers
 ┌────────┐         ┌──────────────────┐       ┌──────────────────┐
 │ ▯ old  │  ──▶    │  old picture,    │  ──▶  │  device's new    │
 │ picture│         │  turned 90°      │       │  pictures        │
 └────────┘         └──────────────────┘       └──────────────────┘
                     refused: turns back, error shows
```

### Consequences

* Good, because a rotation answers the press at once, however far the
  server is.
* Bad, because a refused rotation turns the picture twice.
* Modules touched: the mobile plugin's live session and device view.
