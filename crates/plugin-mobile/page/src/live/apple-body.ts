/**
 * iPhone Duo's own body, Apple's USDZ that Xcode installs for the device
 * type's product class (`./fold-scene.ts`), read on the page from the Host
 * and kept for the session, never stored.
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
 * The model lies in its XZ plane, its inner display towards +Y, the hinge
 * along Z; the scene stands it up in the XY plane.
 */
/// <reference path="./three-usd.d.ts" />
import * as THREE from 'three'
import { USDCParser, type UsdLayer } from 'three/addons/loaders/usd/USDCParser.js'
import { USDComposer } from 'three/addons/loaders/usd/USDComposer.js'
import { unzipSync } from 'three/addons/libs/fflate.module.js'
import type { Body, LitDisplay, Size } from './fold-scene'

/** The stretch of a body's animation that folds it: flat at `from`, closed at `to`, in seconds. */
interface FoldStretch {
  from: number
  to: number
}

/** The stretch measured for each file, by its SHA-256: Xcode 27.1's iPhone Duo. */
const MEASURED: Record<string, FoldStretch> = {
  fd315e57c38fde5666d50de6e19b444e3d145450008126e85838119b682b7940: { from: 5.4167, to: 10.4167 },
}
/** The measured file's animation's length, in seconds, by which a file of another hash is taken alike. */
const MEASURED_LENGTH = 37.5
const MEASURED_STRETCH: FoldStretch = { from: 5.4167, to: 10.4167 }

/** How far a display mesh's aspect may be from its display's, as a share of it. */
const ASPECT_SLACK = 0.06

/** Each body's file as the page read it, by its URL, kept for the session. */
const read = new Map<string, Promise<ArrayBuffer>>()

/** The file at `url`, read once a session. */
export function bodyBytes(url: string): Promise<ArrayBuffer> {
  let bytes = read.get(url)
  if (!bytes) {
    bytes = fetch(url, { credentials: 'include' }).then((response) => {
      if (!response.ok) {
        throw new Error(`the body's file answered ${response.status}`)
      }
      return response.arrayBuffer()
    })
    // A read that failed is tried again by the next view.
    bytes.catch(() => read.delete(url))
    read.set(url, bytes)
  }
  return bytes
}

/** The hex SHA-256 of `bytes`; none where the page's context has no Web Crypto. */
export async function sha256(bytes: ArrayBuffer): Promise<string | null> {
  if (!globalThis.crypto?.subtle) {
    return null
  }
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('')
}

/** The body in `bytes`, a USDZ, for displays of `outer` and `inner` points; none when it does not fit. */
export async function appleBody(bytes: ArrayBuffer, hash: string | null, outer: Size, inner: Size): Promise<Body | null> {
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
  const composer = new USDComposer()
  const model = composer.compose(layer, assets, {}, layerName.slice(0, layerName.lastIndexOf('/')))
  await Promise.all(composer.texturePromises)
  const clip = model.animations[0]
  const stretch = (hash ? MEASURED[hash] : undefined)
    ?? (clip && Math.abs(clip.duration - MEASURED_LENGTH) < 0.01 ? MEASURED_STRETCH : undefined)
  if (!clip || !stretch) {
    return null
  }
  const mixer = new THREE.AnimationMixer(model)
  const action = mixer.clipAction(clip)
  action.play()
  action.paused = true
  const pose = (degrees: number) => {
    action.time = stretch.from + ((180 - degrees) / 180) * (stretch.to - stretch.from)
    mixer.update(0)
    model.updateMatrixWorld(true)
  }
  pose(180)
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
  return {
    object,
    flat: { width: whole.x, height: whole.y },
    displays: { inner: [found.inner], outer: [found.outer] },
    fold: pose,
    dispose() {
      mixer.stopAllAction()
      model.traverse((part) => {
        if (part instanceof THREE.Mesh) {
          part.geometry.dispose()
          for (const material of [part.material].flat()) {
            material.dispose()
          }
        }
      })
    },
  }
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
