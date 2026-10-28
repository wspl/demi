/**
 * Reads iPhone Duo's body off the page's thread (`./apple-body.ts`): unzips
 * Apple's USDZ, parses and composes its model layer with three.js, poses it
 * flat, finds its two displays and lays it out, then hands it to the page as
 * plain data (`./body-transfer.ts`).
 *
 * - The package's first layer only picks a color variant, which three.js's
 *   composer cannot read; the body is its model layer, in its own finish.
 * - Its names are scrambled, so its two displays are found by their shape:
 *   flat, with the aspect of the device's displays.
 * - Its one animation folds the hinge over a stretch of its time, flat to
 *   closed, measured for the file of a known SHA-256, and taken alike from
 *   a file of another hash whose animation is as long. Any other file does
 *   not fit, and the view keeps its stand-in.
 *
 * A worker has no `Image`, so the composer's textures are read as
 * `ImageBitmap`s, turned upright as they are read, since WebGL does not
 * flip a bitmap as it flips an image.
 */
/// <reference path="./three-usd.d.ts" />
import * as THREE from 'three'
import { USDCParser, type UsdLayer } from 'three/addons/loaders/usd/USDCParser.js'
import { USDComposer } from 'three/addons/loaders/usd/USDComposer.js'
import { unzipSync } from 'three/addons/libs/fflate.module.js'
import { encode, type ModelData } from './body-transfer'
import type { LitDisplay, Size } from './fold-scene'

/** The stretch of a body's animation that folds it: flat at `from`, closed at `to`, in seconds. */
export interface FoldStretch {
  from: number
  to: number
}

/** What the page asks: the file's bytes and the device's displays, in points. */
export interface BodyRequest {
  bytes: ArrayBuffer
  outer: Size
  inner: Size
}

/** The body laid out: the model, its fold, its displays' nodes and its open size. */
export interface BodyAnswer {
  model: ModelData
  stretch: FoldStretch
  /** Each display's index among the model's nodes, in the order `traverse` visits them. */
  displays: Record<LitDisplay, number>
  flat: Size
}

/** The worker's answer: the body, none for a file that does not fit, or why it failed. */
export type BodyReply = { body: BodyAnswer | null } | { error: string }

/** The stretch measured for each file, by its SHA-256: Xcode 27.1's iPhone Duo. */
const MEASURED: Record<string, FoldStretch> = {
  fd315e57c38fde5666d50de6e19b444e3d145450008126e85838119b682b7940: { from: 5.4167, to: 10.4167 },
}
/** The measured file's animation's length, in seconds, by which a file of another hash is taken alike. */
const MEASURED_LENGTH = 37.5
const MEASURED_STRETCH: FoldStretch = { from: 5.4167, to: 10.4167 }

/** How far a display mesh's aspect may be from its display's, as a share of it. */
const ASPECT_SLACK = 0.06

/** The composer, its textures read as bitmaps. */
class BitmapComposer extends USDComposer {
  override _createTextureFromData(
    data: Uint8Array | ArrayBuffer | string | null,
    textureAttrs: Record<string, unknown> | undefined,
    transformAttrs: Record<string, unknown> | undefined,
  ): THREE.Texture | null {
    if (!data || typeof data === 'string') {
      return null
    }
    const texture = new THREE.Texture()
    this.texturePromises.push(
      createImageBitmap(new Blob([data as BlobPart]), { imageOrientation: 'flipY', premultiplyAlpha: 'none' }).then((bitmap) => {
        texture.image = bitmap
        if (textureAttrs) {
          texture.wrapS = this._getWrapMode(textureAttrs['inputs:wrapS'])
          texture.wrapT = this._getWrapMode(textureAttrs['inputs:wrapT'])
        }
        this._applyTextureTransforms(texture, transformAttrs)
        texture.needsUpdate = true
      }),
    )
    return texture
  }
}

self.onmessage = (event: MessageEvent<BodyRequest>) => {
  read(event.data).then(
    (read) => {
      if (read) {
        self.postMessage({ body: read.body } satisfies BodyReply, { transfer: read.transfer })
      } else {
        self.postMessage({ body: null } satisfies BodyReply)
      }
    },
    (error: unknown) => self.postMessage({ error: String(error) } satisfies BodyReply),
  )
}

/** The body in `bytes`, a USDZ, for displays of `outer` and `inner` points; none when it does not fit. */
async function read({ bytes, outer, inner }: BodyRequest): Promise<{ body: BodyAnswer; transfer: Transferable[] } | null> {
  const hash = await sha256(bytes)
  const files = unzipSync(new Uint8Array(bytes))
  const names = Object.keys(files)
  const assets: Record<string, UsdLayer | Uint8Array> = {}
  for (const name of names) {
    const file = files[name]!
    if (/\.(png|jpe?g)$/i.test(name)) {
      assets[name] = file
    } else if (name.endsWith('.usdc')) {
      assets[name] = new USDCParser().parseData(file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength) as ArrayBuffer)
    }
  }
  const layerName = names.find((name, index) => index > 0 && name.endsWith('.usdc'))
  const layer = layerName ? assets[layerName] : undefined
  if (!layerName || !layer || layer instanceof Uint8Array) {
    return null
  }
  const composer = new BitmapComposer()
  const model = composer.compose(layer, assets, {}, layerName.slice(0, layerName.lastIndexOf('/')))
  await Promise.all(composer.texturePromises)
  const clip = model.animations[0]
  const stretch = (hash ? MEASURED[hash] : undefined)
    ?? (clip && Math.abs(clip.duration - MEASURED_LENGTH) < 0.01 ? MEASURED_STRETCH : undefined)
  if (!clip || !stretch) {
    return null
  }
  // Flat, as the displays are found and the body is measured.
  const mixer = new THREE.AnimationMixer(model)
  const action = mixer.clipAction(clip)
  action.play()
  action.paused = true
  action.time = stretch.from
  mixer.update(0)
  model.updateMatrixWorld(true)
  const found = displays(model, outer, inner)
  if (!found) {
    return null
  }
  // Laid out in the displays' points, standing, the inner display towards +Z.
  const object = new THREE.Group()
  const innerBox = new THREE.Box3().setFromObject(found.inner, true)
  const scale = inner.width / innerBox.getSize(new THREE.Vector3()).x
  model.rotation.x = Math.PI / 2
  model.scale.setScalar(scale)
  object.add(model)
  const whole = new THREE.Box3().setFromObject(model, true).getSize(new THREE.Vector3())
  const nodes: THREE.Object3D[] = []
  object.traverse((node) => nodes.push(node))
  const { data, transfer } = encode(object, clip)
  return {
    body: {
      model: data,
      stretch,
      displays: { inner: nodes.indexOf(found.inner), outer: nodes.indexOf(found.outer) },
      flat: { width: whole.x, height: whole.y },
    },
    transfer,
  }
}

/** The hex SHA-256 of `bytes`; none where the worker's context has no Web Crypto. */
async function sha256(bytes: ArrayBuffer): Promise<string | null> {
  if (!globalThis.crypto?.subtle) {
    return null
  }
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('')
}

/**
 * The model's two displays laid flat: the largest plane of each display's
 * aspect, the outer one under the inner. A display is a plane with no
 * depth; the glass over it, of nearly its aspect and larger, has some.
 */
function displays(model: THREE.Object3D, outer: Size, inner: Size): Record<LitDisplay, THREE.Mesh> | null {
  const flat: { mesh: THREE.Mesh; size: THREE.Vector3; center: THREE.Vector3 }[] = []
  model.traverse((part) => {
    if (!(part instanceof THREE.Mesh)) {
      return
    }
    const box = new THREE.Box3().setFromObject(part, true)
    const size = box.getSize(new THREE.Vector3())
    if (size.x > 0 && size.z > 0 && size.y <= Math.max(size.x, size.z) * 1e-4) {
      flat.push({ mesh: part, size, center: box.getCenter(new THREE.Vector3()) })
    }
  })
  const largest = (display: Size, keep: (center: THREE.Vector3) => boolean) => {
    const aspect = display.width / display.height
    return flat
      .filter(({ size, center }) => Math.abs(size.x / size.z - aspect) <= aspect * ASPECT_SLACK && keep(center))
      .sort((a, b) => b.size.x * b.size.z - a.size.x * a.size.z)[0]
  }
  const innerDisplay = largest(inner, () => true)
  if (!innerDisplay) {
    return null
  }
  const outerDisplay = largest(outer, (center) => center.y < innerDisplay.center.y)
  return outerDisplay ? { inner: innerDisplay.mesh, outer: outerDisplay.mesh } : null
}
