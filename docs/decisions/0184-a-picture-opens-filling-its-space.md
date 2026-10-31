---
status: proposed
---

# A picture or a video opens filling its space

## Problem

Alex opens a generated 1024 × 1024 picture from a reply. It shows in the
work panel's Media tab, which Alex has widened to 660 px, on a display of two
device pixels per page pixel. The picture takes the middle 512 × 512 px of the
panel, with a wide empty band around it, and Alex has to zoom to see it at the
size the panel offers.

A picture opens fitted, but fitted only scales it down: its largest size is its
actual size, one of its pixels on one device pixel, which on such a display is
half its pixel size in page pixels. A small picture, or a video narrower than
its frame, stays small the same way.

## Decision

A picture opens as large as its frame lets it, enlarged when it is smaller, as
Photos and Google Photos open one. A video does the same in its player.

```text
 Media tab, 660 px wide           before          after
 1024 × 1024 picture              512 × 512        660 × 660
 400 × 300 picture                200 × 150        660 × 495
```

- A viewer's fitted scale is `fillScale` in `image-zoom.ts`, the frame's size
  over the picture's on the tighter axis, with no cap. `fitScale` keeps its
  cap of 1 for what only scales down: a thumbnail's tile and a picture's hover
  card.
- The actual size keeps its meaning, one of the picture's pixels on one device
  pixel. A click still toggles the fitted and the actual size, so on a
  picture fitted larger than its actual size it shows the actual size, smaller.
- The zoom runs from the fitted or the actual size, whichever is smaller, to
  eight times the actual size or the fitted size, whichever is larger. The
  zoom cursor says which way a click goes.
- A video's player in `MediaPreview` takes the video's fitted size, so its
  controls stay on the video and a tap beside it stays the stage's, as on a
  phone, where it closes the medium.

It holds wherever `ImagePreview` and `MediaPreview` show a medium: the Media
tab, the media viewer's window and full screen, the File view and the Change
view. No stored data or protocol changes.

## Spike

None.

## Prior art

- Apple Photos and Google Photos open a picture fitted to the window, a small
  one enlarged.
- A browser's own image document, macOS Preview and Quick Look scale a large
  picture down and never enlarge a small one, with one picture pixel on one
  page pixel or point. The user weighed this rule and chose to fill the space.

## Alternatives

- **Scale down only, one picture pixel on one page pixel.** A picture of 1024
  pixels or more fills the panel, and a small one stays sharp at its size,
  but a small picture still leaves the panel mostly empty.
- **Keep the current rule.** A picture shows at half its pixel size on a
  high-density display, the case above.

## Open questions

None.
