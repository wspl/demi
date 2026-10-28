/**
 * iPhone Duo's own body, Apple's USDZ that Xcode installs for the device
 * type's product class (`./fold-scene.ts`), read on the page from the Host
 * and kept for the session, never stored.
 *
 * A worker reads it (`./apple-body-worker.ts`), so the page never waits on
 * its unzipping, parsing and composing, once a session for each file and
 * device: every view after the first takes a copy of the body the first
 * read, and poses it on a mixer of its own. A copy has geometries, materials
 * and textures of its own that share the read body's arrays and images, so
 * nothing large is copied, and a view that goes disposes of them: a renderer
 * holds on to each resource it drew until that resource is disposed of, and
 * a resource the session kept and every view shared would hold every
 * renderer the session ever made.
 *
 * The model lies in its XZ plane, its inner display towards +Y, the hinge
 * along Z; the worker stands it up in the XY plane.
 */
import * as THREE from 'three'
import { clone } from 'three/addons/utils/SkeletonUtils.js'
import { decode } from './body-transfer'
import type { BodyReply, BodyRequest, FoldStretch } from './apple-body-worker'
import type { Body, Size } from './fold-scene'

/** A body as the session keeps it, which each view copies. */
interface Kept {
  object: THREE.Object3D
  clip: THREE.AnimationClip
  stretch: FoldStretch
  displays: { inner: number; outer: number }
  flat: Size
}

/** Each body the session read, by its file's URL and the device's displays. */
const kept = new Map<string, Promise<Kept | null>>()

/** The body in the file at `url`, for displays of `outer` and `inner` points; none when it does not fit. */
export async function appleBody(url: string, outer: Size, inner: Size): Promise<Body | null> {
  const key = `${url} ${outer.width}x${outer.height} ${inner.width}x${inner.height}`
  let body = kept.get(key)
  if (!body) {
    body = read(url, outer, inner)
    // A read that failed is tried again by the next view.
    body.catch(() => kept.delete(key))
    kept.set(key, body)
  }
  const found = await body
  return found && copy(found)
}

async function read(url: string, outer: Size, inner: Size): Promise<Kept | null> {
  const response = await fetch(url, { credentials: 'include' })
  if (!response.ok) {
    throw new Error(`the body's file answered ${response.status}`)
  }
  const bytes = await response.arrayBuffer()
  const worker = new Worker(new URL('./apple-body-worker.ts', import.meta.url), { type: 'module' })
  try {
    const reply = await new Promise<BodyReply>((resolve, reject) => {
      worker.onmessage = (event: MessageEvent<BodyReply>) => resolve(event.data)
      worker.onerror = (event) => reject(new Error(event.message || 'the worker that reads the body failed'))
      worker.onmessageerror = () => reject(new Error('the body could not be read from the worker'))
      // The sizes may be a view's reactive props, which a message cannot carry.
      const request: BodyRequest = {
        bytes,
        outer: { width: outer.width, height: outer.height },
        inner: { width: inner.width, height: inner.height },
      }
      worker.postMessage(request, [bytes])
    })
    if ('error' in reply) {
      throw new Error(reply.error)
    }
    if (!reply.body) {
      return null
    }
    const { model, clip } = decode(reply.body.model)
    if (!clip) {
      return null
    }
    return { object: model, clip, stretch: reply.body.stretch, displays: reply.body.displays, flat: reply.body.flat }
  } finally {
    worker.terminate()
  }
}

/** A view's copy of `body`, posed flat. */
function copy(body: Kept): Body {
  const object = clone(body.object)
  const nodes: THREE.Object3D[] = []
  object.traverse((node) => nodes.push(node))
  const owned = own(object)
  const model = object.children[0]!
  const mixer = new THREE.AnimationMixer(model)
  const action = mixer.clipAction(body.clip)
  action.play()
  action.paused = true
  const { stretch } = body
  const fold = (degrees: number) => {
    action.time = stretch.from + ((180 - degrees) / 180) * (stretch.to - stretch.from)
    mixer.update(0)
    model.updateMatrixWorld(true)
  }
  fold(180)
  return {
    object,
    flat: body.flat,
    displays: { inner: [nodes[body.displays.inner] as THREE.Mesh], outer: [nodes[body.displays.outer] as THREE.Mesh] },
    fold,
    dispose() {
      mixer.stopAllAction()
      mixer.uncacheRoot(model)
      for (const resource of owned) {
        resource.dispose()
      }
    },
  }
}

/**
 * Gives each mesh of `object` a geometry, materials and textures of its own,
 * over the same attributes and images; answers them, for the view to
 * dispose of.
 */
function own(object: THREE.Object3D): { dispose(): void }[] {
  const geometries = new Map<THREE.BufferGeometry, THREE.BufferGeometry>()
  const materials = new Map<THREE.Material, THREE.Material>()
  const textures = new Map<THREE.Texture, THREE.Texture>()
  const geometryOf = (shared: THREE.BufferGeometry) => {
    let geometry = geometries.get(shared)
    if (!geometry) {
      geometry = new THREE.BufferGeometry()
      for (const [name, attribute] of Object.entries(shared.attributes)) {
        geometry.setAttribute(name, attribute)
      }
      geometry.setIndex(shared.index)
      for (const group of shared.groups) {
        geometry.addGroup(group.start, group.count, group.materialIndex)
      }
      geometries.set(shared, geometry)
    }
    return geometry
  }
  const materialOf = (shared: THREE.Material) => {
    let material = materials.get(shared)
    if (!material) {
      material = shared.clone()
      const fields = material as unknown as Record<string, unknown>
      for (const [key, value] of Object.entries(fields)) {
        if (value instanceof THREE.Texture) {
          let texture = textures.get(value)
          if (!texture) {
            // A clone shares its image.
            texture = value.clone()
            textures.set(value, texture)
          }
          fields[key] = texture
        }
      }
      materials.set(shared, material)
    }
    return material
  }
  object.traverse((node) => {
    if (node instanceof THREE.Mesh) {
      node.geometry = geometryOf(node.geometry)
      const material = node.material as THREE.Material | THREE.Material[]
      node.material = Array.isArray(material) ? material.map(materialOf) : materialOf(material)
    }
  })
  return [...geometries.values(), ...materials.values(), ...textures.values()]
}
