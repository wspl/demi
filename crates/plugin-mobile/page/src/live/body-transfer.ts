/**
 * A three.js model as plain data a worker hands the page (`./apple-body.ts`):
 * its tree of nodes, its geometry, materials and textures, its skeletons and
 * one animation clip. Every array is a typed array the message transfers, so
 * nothing is copied, and every image an `ImageBitmap`. `encode` runs in the
 * worker, `decode` on the page; one module owns both so they never disagree.
 *
 * It carries what three.js's USD composer makes and nothing else: groups,
 * bones, meshes and skinned meshes, non-interleaved attributes, and
 * materials whose values are numbers, booleans, strings, colors, vectors,
 * Euler angles, number lists and textures, and tracks of numbers. `encode` refuses anything else,
 * so a model it cannot carry is never shown half.
 */
import * as THREE from 'three'

type NodeKind = 'Object3D' | 'Group' | 'Bone' | 'Mesh' | 'SkinnedMesh'

interface NodeData {
  kind: NodeKind
  name: string
  /** The parent's index among the nodes, which come parents first; -1 for the root. */
  parent: number
  position: [number, number, number]
  quaternion: [number, number, number, number]
  scale: [number, number, number]
  visible: boolean
  frustumCulled: boolean
  renderOrder: number
  mesh?: {
    geometry: number
    material: number | number[]
    skin?: { skeleton: number; bindMatrix: number[]; bindMode: THREE.BindMode }
  }
}

interface GeometryData {
  attributes: Record<string, { array: THREE.TypedArray; itemSize: number; normalized: boolean }>
  index: THREE.TypedArray | null
  groups: { start: number; count: number; materialIndex?: number }[]
}

type MaterialValue =
  | number
  | boolean
  | string
  | null
  | number[]
  | { texture: number }
  | { color: [number, number, number] }
  | { vector2: [number, number] }
  | { vector3: [number, number, number] }
  | { euler: [number, number, number, THREE.EulerOrder] }

interface MaterialData {
  type: string
  values: Record<string, MaterialValue>
}

/** A texture's image and the settings its material reads. */
interface TextureData {
  image: ImageBitmap
  values: Record<string, number | string | boolean | [number, number]>
  /** Its UV transform, which a composer may set itself rather than from the values. */
  matrix: number[]
}

interface TrackData {
  name: string
  type: string
  times: THREE.TypedArray
  values: THREE.TypedArray
  interpolation: THREE.InterpolationModes
}

export interface ModelData {
  nodes: NodeData[]
  geometries: GeometryData[]
  materials: MaterialData[]
  textures: TextureData[]
  skeletons: { bones: number[]; inverses: Float32Array }[]
  clip: { name: string; duration: number; tracks: TrackData[] } | null
}

/** A texture's settings that travel with it. */
const TEXTURE_VALUES = [
  'wrapS', 'wrapT', 'magFilter', 'minFilter', 'anisotropy', 'colorSpace', 'generateMipmaps',
  'premultiplyAlpha', 'unpackAlignment', 'channel', 'rotation', 'matrixAutoUpdate',
] as const
const TEXTURE_VECTORS = ['offset', 'repeat', 'center'] as const

/** A material's own fields that are not its look. */
const MATERIAL_SKIPPED = new Set(['uuid', 'id', 'type', 'version', 'userData', 'defines', 'listeners'])

/** Thrown for what the data cannot carry. */
export class Uncarried extends Error {}

/** `model` and `clip` as data, and the buffers its message transfers. */
export function encode(model: THREE.Object3D, clip: THREE.AnimationClip | null): { data: ModelData; transfer: Transferable[] } {
  const transfer = new Set<Transferable>()
  const moved = <T extends THREE.TypedArray>(array: T): T => {
    transfer.add(array.buffer as ArrayBuffer)
    return array
  }
  const indexOf = new Map<THREE.Object3D, number>()
  const geometries = new Map<THREE.BufferGeometry, number>()
  const materials = new Map<THREE.Material, number>()
  const textures = new Map<THREE.Texture, number>()
  const skeletons = new Map<THREE.Skeleton, number>()
  const data: ModelData = { nodes: [], geometries: [], materials: [], textures: [], skeletons: [], clip: null }

  const texture = (value: THREE.Texture): number => {
    let index = textures.get(value)
    if (index === undefined) {
      if (!(value.image instanceof ImageBitmap)) {
        throw new Uncarried('a texture whose image is not an ImageBitmap')
      }
      const values: TextureData['values'] = {}
      for (const key of TEXTURE_VALUES) {
        values[key] = value[key] as number | string | boolean
      }
      for (const key of TEXTURE_VECTORS) {
        values[key] = [value[key].x, value[key].y]
      }
      index = data.textures.push({ image: value.image, values, matrix: value.matrix.toArray() }) - 1
      transfer.add(value.image)
      textures.set(value, index)
    }
    return index
  }
  const material = (value: THREE.Material): number => {
    let index = materials.get(value)
    if (index === undefined) {
      const values: Record<string, MaterialValue> = {}
      for (const [own, field] of Object.entries(value)) {
        // A field behind a setter, such as `clearcoat`, is held as `_clearcoat`.
        const key = own.startsWith('_') ? own.slice(1) : own
        if (MATERIAL_SKIPPED.has(key) || key.startsWith('is') || !(key in value) || typeof field === 'function') {
          continue
        }
        values[key] = materialValue(field, texture)
      }
      index = data.materials.push({ type: value.type, values }) - 1
      materials.set(value, index)
    }
    return index
  }
  const geometry = (value: THREE.BufferGeometry): number => {
    let index = geometries.get(value)
    if (index === undefined) {
      const attributes: GeometryData['attributes'] = {}
      for (const [name, attribute] of Object.entries(value.attributes)) {
        if (!(attribute instanceof THREE.BufferAttribute)) {
          throw new Uncarried(`an interleaved attribute, ${name}`)
        }
        attributes[name] = { array: moved(attribute.array), itemSize: attribute.itemSize, normalized: attribute.normalized }
      }
      index = data.geometries.push({
        attributes,
        index: value.index ? moved(value.index.array) : null,
        groups: value.groups.map((group) => ({ ...group })),
      }) - 1
      geometries.set(value, index)
    }
    return index
  }

  model.traverse((node) => {
    const entry: NodeData = {
      kind: kindOf(node),
      name: node.name,
      parent: node === model ? -1 : indexOf.get(node.parent!)!,
      position: node.position.toArray() as NodeData['position'],
      quaternion: node.quaternion.toArray() as NodeData['quaternion'],
      scale: node.scale.toArray() as NodeData['scale'],
      visible: node.visible,
      frustumCulled: node.frustumCulled,
      renderOrder: node.renderOrder,
    }
    indexOf.set(node, data.nodes.push(entry) - 1)
    if (node instanceof THREE.Mesh) {
      const used = node.material as THREE.Material | THREE.Material[]
      entry.mesh = {
        geometry: geometry(node.geometry),
        material: Array.isArray(used) ? used.map(material) : material(used),
      }
    }
  })
  // Bones are nodes, so a skin is told once every node has its index.
  model.traverse((node) => {
    if (!(node instanceof THREE.SkinnedMesh)) {
      return
    }
    let index = skeletons.get(node.skeleton)
    if (index === undefined) {
      const bones = node.skeleton.bones.map((bone) => {
        const at = indexOf.get(bone)
        if (at === undefined) {
          throw new Uncarried('a bone outside the model')
        }
        return at
      })
      const inverses = new Float32Array(bones.length * 16)
      node.skeleton.boneInverses.forEach((inverse, at) => inverse.toArray(inverses, at * 16))
      index = data.skeletons.push({ bones, inverses: moved(inverses) }) - 1
      skeletons.set(node.skeleton, index)
    }
    data.nodes[indexOf.get(node)!]!.mesh!.skin = { skeleton: index, bindMatrix: node.bindMatrix.toArray(), bindMode: node.bindMode }
  })
  if (clip) {
    data.clip = {
      name: clip.name,
      duration: clip.duration,
      tracks: clip.tracks.map((track) => {
        if (!(track.values instanceof Float32Array)) {
          throw new Uncarried(`a track of type ${track.ValueTypeName}`)
        }
        return {
          name: track.name,
          type: track.ValueTypeName,
          times: moved(track.times),
          values: moved(track.values),
          interpolation: track.getInterpolation(),
        }
      }),
    }
  }
  return { data, transfer: [...transfer] }
}

/** The model `data` holds, and its clip. */
export function decode(data: ModelData): { model: THREE.Object3D; clip: THREE.AnimationClip | null } {
  const textures = data.textures.map(({ image, values, matrix }) => {
    const texture = new THREE.Texture(image)
    texture.matrix.fromArray(matrix)
    for (const [key, value] of Object.entries(values)) {
      if (Array.isArray(value)) {
        ;(texture[key as (typeof TEXTURE_VECTORS)[number]]).set(value[0], value[1])
      } else {
        ;(texture as unknown as Record<string, unknown>)[key] = value
      }
    }
    // The worker turned the image upright as it read it: an ImageBitmap
    // ignores WebGL's flip.
    texture.flipY = false
    texture.needsUpdate = true
    return texture
  })
  const materials = data.materials.map(({ type, values }) => {
    const Material = (THREE as unknown as Record<string, new () => THREE.Material>)[type]
    if (!Material) {
      throw new Uncarried(`a material of type ${type}`)
    }
    const material = new Material() as unknown as Record<string, unknown>
    for (const [key, value] of Object.entries(values)) {
      setMaterialValue(material, key, value, textures)
    }
    return material as unknown as THREE.Material
  })
  const geometries = data.geometries.map(({ attributes, index, groups }) => {
    const geometry = new THREE.BufferGeometry()
    for (const [name, attribute] of Object.entries(attributes)) {
      geometry.setAttribute(name, new THREE.BufferAttribute(attribute.array, attribute.itemSize, attribute.normalized))
    }
    if (index) {
      geometry.setIndex(new THREE.BufferAttribute(index, 1))
    }
    for (const group of groups) {
      geometry.addGroup(group.start, group.count, group.materialIndex)
    }
    return geometry
  })
  const nodes = data.nodes.map((entry) => {
    let node: THREE.Object3D
    if (entry.mesh) {
      const material = Array.isArray(entry.mesh.material)
        ? entry.mesh.material.map((at) => materials[at]!)
        : materials[entry.mesh.material]!
      const geometry = geometries[entry.mesh.geometry]!
      node = entry.kind === 'SkinnedMesh' ? new THREE.SkinnedMesh(geometry, material) : new THREE.Mesh(geometry, material)
    } else {
      node = entry.kind === 'Bone' ? new THREE.Bone() : entry.kind === 'Group' ? new THREE.Group() : new THREE.Object3D()
    }
    node.name = entry.name
    node.position.fromArray(entry.position)
    node.quaternion.fromArray(entry.quaternion)
    node.scale.fromArray(entry.scale)
    node.visible = entry.visible
    node.frustumCulled = entry.frustumCulled
    node.renderOrder = entry.renderOrder
    return node
  })
  data.nodes.forEach((entry, at) => {
    if (entry.parent >= 0) {
      nodes[entry.parent]!.add(nodes[at]!)
    }
  })
  const skeletons = data.skeletons.map(({ bones, inverses }) => new THREE.Skeleton(
    bones.map((at) => nodes[at] as THREE.Bone),
    bones.map((_, at) => new THREE.Matrix4().fromArray(inverses, at * 16)),
  ))
  const model = nodes[0]!
  model.updateMatrixWorld(true)
  data.nodes.forEach((entry, at) => {
    const skin = entry.mesh?.skin
    if (skin) {
      const mesh = nodes[at] as THREE.SkinnedMesh
      mesh.bindMode = skin.bindMode
      mesh.bind(skeletons[skin.skeleton]!, new THREE.Matrix4().fromArray(skin.bindMatrix))
    }
  })
  const clip = data.clip && new THREE.AnimationClip(data.clip.name, data.clip.duration, data.clip.tracks.map(track))
  return { model, clip }
}

function kindOf(node: THREE.Object3D): NodeKind {
  if (node instanceof THREE.SkinnedMesh) {
    return 'SkinnedMesh'
  }
  if (node instanceof THREE.Mesh) {
    return 'Mesh'
  }
  if (node instanceof THREE.Bone) {
    return 'Bone'
  }
  if (node instanceof THREE.Group) {
    return 'Group'
  }
  if (node.constructor === THREE.Object3D) {
    return 'Object3D'
  }
  throw new Uncarried(`a node of type ${node.type}`)
}

function materialValue(value: unknown, texture: (value: THREE.Texture) => number): MaterialValue {
  if (value === null || typeof value === 'number' || typeof value === 'boolean' || typeof value === 'string') {
    return value
  }
  if (value instanceof THREE.Texture) {
    return { texture: texture(value) }
  }
  if (value instanceof THREE.Color) {
    return { color: [value.r, value.g, value.b] }
  }
  if (value instanceof THREE.Vector2) {
    return { vector2: [value.x, value.y] }
  }
  if (value instanceof THREE.Vector3) {
    return { vector3: [value.x, value.y, value.z] }
  }
  if (value instanceof THREE.Euler) {
    return { euler: [value.x, value.y, value.z, value.order] }
  }
  if (Array.isArray(value) && value.every((each) => typeof each === 'number')) {
    return [...value]
  }
  throw new Uncarried(`a material value ${String(value)}`)
}

function setMaterialValue(material: Record<string, unknown>, key: string, value: MaterialValue, textures: THREE.Texture[]): void {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    material[key] = value
  } else if ('texture' in value) {
    material[key] = textures[value.texture]
  } else if ('color' in value) {
    material[key] = new THREE.Color().setRGB(...value.color)
  } else if ('vector2' in value) {
    material[key] = new THREE.Vector2(...value.vector2)
  } else if ('vector3' in value) {
    material[key] = new THREE.Vector3(...value.vector3)
  } else {
    material[key] = new THREE.Euler(...value.euler)
  }
}

function track({ name, type, times, values, interpolation }: TrackData): THREE.KeyframeTrack {
  const tracks: Record<string, new (name: string, times: ArrayLike<number>, values: ArrayLike<number>, interpolation?: THREE.InterpolationModes) => THREE.KeyframeTrack> = {
    quaternion: THREE.QuaternionKeyframeTrack,
    vector: THREE.VectorKeyframeTrack,
    number: THREE.NumberKeyframeTrack,
    color: THREE.ColorKeyframeTrack,
  }
  const Track = tracks[type]
  if (!Track) {
    throw new Uncarried(`a track of type ${type}`)
  }
  return new Track(name, times, values, interpolation)
}
