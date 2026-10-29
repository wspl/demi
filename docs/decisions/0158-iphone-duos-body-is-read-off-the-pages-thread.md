---
status: proposed
---

# iPhone Duo's body is read off the page's thread, once a session, and shows without a stand-in first

## Problem

Alex opens iPhone Duo in the work panel. The view shows the stand-in of two
slabs, a device that looks like no iPhone, then Apple's body replaces it,
and as it does the page stops: a task
of about 124 ms that unzips, parses and composes Apple's USDZ, then frames of
67 to 100 ms that put its textures and its 15 MB of vertices on the GPU. A
tap or a drag in that moment waits. Alex reloads the page, switches to another
conversation and back, or closes and opens the device's tab: each time the
view reads the same file again, from its bytes, which the page keeps for the
session, and stops the page again.

## Decision

Apple's body is read in a worker, once a session for each file and device,
and every view of the session takes a copy of it. While the body is on its
way, the view draws no device: the device's status shows over the empty
view, as before the first picture, and Apple's body is the first device it
draws. The stand-in shows only where Apple's body cannot: Xcode has none, or
its file does not fit.

### A worker reads the file

`crates/plugin-mobile/page/src/live/apple-body-worker.ts` runs three.js's
USDZ reading (`unzipSync`, `USDCParser`, `USDComposer`), poses the body flat,
finds its two displays and lays it out, as `apple-body.ts` does today, with
one change: a texture's image is an `ImageBitmap` that the worker decodes,
turned upright as it is read, since a worker has no `Image` and WebGL does
not flip a bitmap. The page fetches the file and transfers its bytes to the
worker, so the page's service worker still serves it over the direct
channel.

The worker answers the laid-out body as plain data whose arrays and images
move to the page without a copy (`body-transfer.ts`, which owns both the
writing and the reading):

```ts
interface ModelData {
  /** Parents first, each with its transform; a mesh names its geometry, materials and skin. */
  nodes: NodeData[]
  geometries: GeometryData[]
  /** A material's type and its values: numbers, colors, vectors and texture indexes. */
  materials: MaterialData[]
  textures: { image: ImageBitmap; values: Record<string, …>; matrix: number[] }[]
  skeletons: { bones: number[]; inverses: Float32Array }[]
  clip: { name: string; duration: number; tracks: TrackData[] } | null
}
```

It carries what three.js's USD composer makes: groups, bones, meshes and
skinned meshes, plain attributes, and tracks of numbers. A model with
anything else is refused whole, and the view keeps its stand-in. The page
builds the three.js objects from it, which takes no time worth measuring:
an attribute wraps its array.

### Once a session

The page keeps one body for each file and device for the session: the
first view that needs it starts the worker, and every view after takes a
copy (`SkeletonUtils.clone`), posed by a mixer of its own. A copy's
geometries, materials and textures are its own but wrap the kept body's
attributes and images, so nothing large is copied, and a view that goes
disposes of them. A renderer holds on to every resource it drew until that
resource is disposed of, so resources every view shared, which the session
never disposes of, would hold every renderer the session made.

### The GPU's part, a little in each frame

Before the body replaces the stand-in, its shaders compile without holding a
frame (`compileAsync`, as the simulator view's fix does), and its textures go
to the GPU as many in each frame as 6 ms allows (`initTexture`), so no frame
carries them all. The vertices go with the first frame that draws the body,
which three.js offers no way to split; that frame stays.

## Spike

On this Mac (M5 Max), in Chrome 153, with Xcode 27.1's `V68.usdz` (5.9 MB):

| Step on the page's thread today        | Time            |
| -------------------------------------- | --------------- |
| Unzip                                  | 1 ms            |
| Parse the USDC layers                  | 5–24 ms         |
| Compose the model                      | 31–47 ms        |
| Upload its 42 textures (`initTexture`) | 19 ms           |
| First draw, shaders compiled before    | 46 ms           |
| First draw, shaders not compiled       | 174 ms; 2.5 s with no shader cache |

- `USDComposer` composed the body in a worker in 59–66 ms once its textures
  were read with `createImageBitmap`: 73 skinned meshes, one clip, 15.1 MB of
  vertex data, all of it in typed arrays a worker can transfer.
- In the product, a reload shows Apple's body with one frame of 73 to 79 ms,
  its first draw, where it had a 124 ms task and frames of 67 to 100 ms. A
  view opened again, as on returning to the conversation, starts no worker.
- The page's HTTP cache already keeps the file's bytes: its route answers
  with its version as the ETag and `private, no-cache`, so a page that loads
  again asks whether it changed and gets `304 Not Modified`. A file read over
  the direct channel goes through the page's service worker, which the HTTP
  cache does not see, but there the Host is close and the file comes in
  about 15 ms.

## Prior art

- three.js's [`DRACOLoader`](https://github.com/mrdoob/three.js/blob/r186/examples/jsm/loaders/DRACOLoader.js)
  decodes in a pool of workers and transfers typed arrays back; the page
  builds the `BufferGeometry` from them. Demi does the same for a whole
  composed model.
- [loaders.gl](https://github.com/visgl/loaders.gl) parses in workers into
  plain data with transferred arrays, which the application turns into its
  own objects.
- three.js's [`SkeletonUtils.clone`](https://github.com/mrdoob/three.js/blob/r186/examples/jsm/utils/SkeletonUtils.js)
  copies a skinned model sharing its geometry and materials, as an
  application that shows one model many times does.

## Alternatives

- Draw the whole 3D view in a worker on an `OffscreenCanvas`, as three.js's
  `webgl_worker_offscreencanvas` example does: the displays' pictures are
  decoded on the page and would cross to the worker each frame, and every
  pointer move would wait for the worker to say where it fell.
- Parse in the worker and compose on the page: the composition is the
  larger half.
- Serialize with three.js's `toJSON` and `ObjectLoader`: it writes every
  array as a JSON list, slower than reading the file.
- Keep the worker's answer in the page's IndexedDB beyond the session, keyed
  by the file's SHA-256: a reload would show Apple's body about 70 ms sooner,
  and Apple's model, which 0133 keeps for the session only, would stay in
  the browser of every computer and phone that opened the view. The HTTP
  cache already keeps the bytes, and the worker removes the pause.

## Open questions

None.
