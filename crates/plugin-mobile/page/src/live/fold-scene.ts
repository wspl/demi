/**
 * A hinged foldable in 3D, for the device view of iPhone Duo: the device's
 * body, each display's live picture on its own face, as Xcode's Device Hub
 * shows them, the hinge at an angle and the view turned about the device's
 * upright axis. A display wears its own dark finish until its first picture,
 * and the one the guest lit takes the pointer.
 *
 * The body is Apple's own, the USDZ Xcode installs (`./apple-body.ts`), or,
 * where there is none or it does not fit, a stand-in of two slabs. Either
 * lies in the XY plane in the displays' points, its inner display towards
 * +Z, the hinge along Y at x = 0. The right half stays; the left half turns
 * about the hinge, so at 0° it lies on the right one, its back, which holds
 * the outer display, towards the viewer. The camera faces the displays from
 * between the halves, as both pages of a book read at once, turned further
 * by the viewer's drag. A device held sideways shows turned about the view's
 * axis, as a 2D frame turns, and the picture, upright for the viewer, turned
 * back onto its display.
 *
 *        hinge
 *   left   |   right            0°: left on right, outer display up
 *  ------  |  ------          130°: half open, pages 25° each way
 *          |                  180°: flat, the inner display whole
 */
import * as THREE from 'three'
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js'
import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js'

export interface Size {
  width: number
  height: number
}

export type LitDisplay = 'outer' | 'inner'

/** How far the viewer may turn the view either way, in degrees: past it the display is too slanted to read or tap. */
export const MAX_YAW = 60

/** A point of the picture under the pointer, as fractions of its width and height from its top left. */
export interface DisplayPoint {
  x: number
  y: number
}

/** What the scene draws the device as. */
export interface Body {
  /** The device as the scene lays it out. */
  readonly object: THREE.Object3D
  /** The open device's size, which the camera fits. */
  readonly flat: Size
  /** Each display's meshes, whose uv maps the picture upright onto them. */
  readonly displays: Record<LitDisplay, THREE.Mesh[]>
  /** Folds the hinge to `degrees`. */
  fold(degrees: number): void
  dispose(): void
}

/** The stand-in's bezel around each display and depth, in the displays' points. */
const BEZEL = 14
const HALF_DEPTH = 9
const CORNER = 34

/** Two slabs and their displays, sized from the device's displays. */
export function standIn(outer: Size, inner: Size): Body {
  const object = new THREE.Group()
  const left = new THREE.Group()
  const half = { width: inner.width / 2 + BEZEL, height: inner.height + BEZEL * 2 }
  const material = new THREE.MeshPhysicalMaterial({ color: 0x2a2c30, metalness: 0.85, roughness: 0.32, clearcoat: 0.3 })
  const geometries: THREE.BufferGeometry[] = []
  const slab = () => {
    const geometry = new RoundedBoxGeometry(half.width, half.height, HALF_DEPTH, 4, CORNER / 3)
    geometries.push(geometry)
    return new THREE.Mesh(geometry, material)
  }
  // A display's part on `parent`: `width` × `height` at x `at`, showing the
  // picture's horizontal stretch from `from` to `to`, on the front or on the
  // half's back.
  const display = (parent: THREE.Object3D, width: number, height: number, at: number, from: number, to: number, back = false) => {
    const geometry = new THREE.PlaneGeometry(width, height)
    geometries.push(geometry)
    const uv = geometry.attributes.uv!
    for (let i = 0; i < uv.count; i += 1) {
      uv.setX(i, from + (to - from) * uv.getX(i))
    }
    const mesh = new THREE.Mesh(geometry)
    if (back) {
      mesh.rotation.y = Math.PI
      mesh.position.set(at, 0, -HALF_DEPTH - 0.5)
    } else {
      mesh.position.set(at, 0, 0.5)
    }
    parent.add(mesh)
    return mesh
  }
  const right = slab()
  right.position.set(half.width / 2, 0, -HALF_DEPTH / 2)
  object.add(right)
  const leftSlab = slab()
  leftSlab.position.set(-half.width / 2, 0, -HALF_DEPTH / 2)
  left.add(leftSlab)
  object.add(left)
  const displays: Record<LitDisplay, THREE.Mesh[]> = {
    inner: [
      display(left, inner.width / 2, inner.height, -inner.width / 4, 0, 0.5),
      display(object, inner.width / 2, inner.height, inner.width / 4, 0.5, 1),
    ],
    outer: [display(left, outer.width, outer.height, -half.width / 2, 0, 1, true)],
  }
  return {
    object,
    flat: { width: inner.width + BEZEL * 2, height: inner.height + BEZEL * 2 },
    displays,
    fold(degrees) {
      left.rotation.y = THREE.MathUtils.degToRad(180 - degrees)
    },
    dispose() {
      for (const geometry of geometries) {
        geometry.dispose()
      }
      material.dispose()
    },
  }
}

/** A display's live picture on its face. */
interface Face {
  readonly picture: HTMLCanvasElement
  texture: THREE.CanvasTexture
  /** The picture's size the texture holds: WebGL keeps a texture's size, so a new size takes a new texture. */
  size: Size
  readonly material: THREE.MeshBasicMaterial
  /** Whether it wears the picture, which it does from the picture's first paint. */
  worn: boolean
}

export class FoldScene {
  private readonly renderer: THREE.WebGLRenderer
  private readonly scene = new THREE.Scene()
  private readonly camera = new THREE.PerspectiveCamera(22, 1, 1, 100000)
  private readonly faces: Record<LitDisplay, Face>
  private readonly dark = new THREE.MeshBasicMaterial({ color: 0x050505 })
  /** Each display mesh's own material, which it wears while dark. */
  private readonly own = new Map<THREE.Mesh, THREE.Material | THREE.Material[]>()
  private readonly raycaster = new THREE.Raycaster()
  private body: Body
  private disposed = false
  private size: Size = { width: 1, height: 1 }
  private degrees = 180
  private yaw = 0
  /** Quarter turns clockwise the device is held from upright. */
  private quarters = 0
  private shown: LitDisplay = 'inner'
  /**
   * The body's meshes as they stand at the hinge's angle `degrees`, which
   * the pointer's rays test, each with the mesh it stands for. A skinned
   * mesh's copy holds its vertices posed: three.js moves every vertex by its
   * bones at each ray otherwise, some 20 ms of Apple's body at every move of
   * the pointer, which held the page's frames back.
   */
  private posed: { degrees: number; meshes: Map<THREE.Mesh, THREE.Mesh> } | null = null

  constructor(
    readonly canvas: HTMLCanvasElement,
    pictures: Record<LitDisplay, HTMLCanvasElement>,
    outer: Size,
    inner: Size,
  ) {
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true })
    this.renderer.toneMapping = THREE.ACESFilmicToneMapping
    this.renderer.outputColorSpace = THREE.SRGBColorSpace
    // A room to reflect, so the body's metal does not show black.
    const environment = new THREE.PMREMGenerator(this.renderer).fromScene(new RoomEnvironment(), 0.04)
    this.scene.environment = environment.texture
    const face = (picture: HTMLCanvasElement): Face => {
      const texture = this.pictureTexture(picture)
      const material = new THREE.MeshBasicMaterial({ map: texture, toneMapped: false })
      return { picture, texture, size: { width: picture.width, height: picture.height }, material, worn: false }
    }
    this.faces = { outer: face(pictures.outer), inner: face(pictures.inner) }
    this.body = standIn(outer, inner)
    this.wear(this.body)
  }

  /**
   * Draws the device as `body` from now on, as Apple's once it loaded, once
   * its shaders are compiled: compiled at its first draw, they held that
   * frame back for a quarter of a second. Answers whether the scene took it,
   * which a scene disposed of meanwhile does not.
   */
  async useBody(body: Body): Promise<boolean> {
    await this.renderer.compileAsync(body.object, this.camera, this.scene)
    if (this.disposed) {
      return false
    }
    this.scene.remove(this.body.object)
    this.body.dispose()
    this.own.clear()
    this.posed = null
    this.body = body
    this.wear(body)
    return true
  }

  private wear(body: Body): void {
    for (const mesh of [...body.displays.inner, ...body.displays.outer]) {
      // A stand-in's display has no finish of its own: dark is black.
      this.own.set(mesh, Array.isArray(mesh.material) || mesh.material.type !== 'MeshBasicMaterial' ? mesh.material : this.dark)
    }
    this.scene.add(body.object)
    body.fold(this.degrees)
    for (const display of ['outer', 'inner'] as const) {
      this.wearFace(display)
    }
  }

  /** `display`'s meshes wear its picture once it has one, else their own dark finish. */
  private wearFace(display: LitDisplay): void {
    const face = this.faces[display]
    for (const mesh of this.body.displays[display]) {
      mesh.material = face.worn ? face.material : this.own.get(mesh) ?? this.dark
    }
  }

  setDegrees(degrees: number): void {
    this.degrees = Math.min(180, Math.max(0, degrees))
    this.body.fold(this.degrees)
  }

  setYaw(yaw: number): void {
    this.yaw = Math.min(MAX_YAW, Math.max(-MAX_YAW, yaw))
  }

  /** Holds the device `quarters` quarter turns clockwise from upright, as its screen reports. */
  setTurn(quarters: number): void {
    this.quarters = ((quarters % 4) + 4) % 4
    for (const face of Object.values(this.faces)) {
      this.turnPicture(face.texture)
    }
  }

  /** The picture comes upright for the viewer: on its display it turns back as far as the device turned. */
  private turnPicture(texture: THREE.Texture): void {
    texture.center.set(0.5, 0.5)
    texture.rotation = (this.quarters * Math.PI) / 2
  }

  /** Whether `display`'s picture painted, which its face wears from then on. */
  painted(display: LitDisplay, painted: boolean): void {
    if (this.faces[display].worn !== painted) {
      this.faces[display].worn = painted
      this.wearFace(display)
    }
  }

  /** The display the guest lit, which takes the pointer. */
  light(display: LitDisplay): void {
    this.shown = display
  }

  resize(width: number, height: number, ratio: number): void {
    this.size = { width: Math.max(1, width), height: Math.max(1, height) }
    this.renderer.setPixelRatio(ratio)
    this.renderer.setSize(this.size.width, this.size.height, false)
    this.camera.aspect = this.size.width / this.size.height
    this.camera.updateProjectionMatrix()
  }

  /**
   * The view's own turn at the hinge's angle: from between the two pages
   * while the inner display is lit, from between the outer display and the
   * page under it while the outer one is, so the view turns on without a
   * jump as the device folds. 0° and 180° face the device.
   */
  private baseYaw(): number {
    const turned = 180 - this.degrees
    return Math.min(turned, 180 - turned) / 2
  }

  private pictureTexture(picture: HTMLCanvasElement): THREE.CanvasTexture {
    const texture = new THREE.CanvasTexture(picture)
    texture.colorSpace = THREE.SRGBColorSpace
    // The picture comes about the size the display shows it, and changes
    // every frame: mipmaps, made again for each, would show it from a
    // smaller copy, blurred, wherever it is drawn slanted or a little smaller.
    texture.generateMipmaps = false
    texture.minFilter = THREE.LinearFilter
    this.turnPicture(texture)
    return texture
  }

  /** Draws the device with each picture's latest paint. */
  render(): void {
    for (const display of ['outer', 'inner'] as const) {
      const face = this.faces[display]
      const { width, height } = face.picture
      if (width !== face.size.width || height !== face.size.height) {
        face.texture.dispose()
        face.texture = this.pictureTexture(face.picture)
        face.material.map = face.texture
        face.size = { width, height }
      }
      face.texture.needsUpdate = face.worn
    }
    // The camera keeps one distance, at which the open device fits the view however it turns about its
    // upright axis, so the device keeps its size, and stays whole, in every posture and turn: across,
    // the circle its width sweeps; upright, its height at its nearest edge.
    // A device held sideways lies across the view: its height across, the circle upright.
    const flat = this.body.flat
    const radius = flat.width / 2
    const vertical = THREE.MathUtils.degToRad(this.camera.fov) / 2
    const horizontal = Math.atan(Math.tan(vertical) * this.camera.aspect)
    const [across, upright] = this.quarters % 2 === 0 ? [horizontal, vertical] : [vertical, horizontal]
    const distance = Math.max(radius / Math.sin(across), flat.height / 2 / Math.tan(upright) + radius) * 1.06
    // Clip planes close around the device keep the depth fine enough for a display just above its half.
    this.camera.near = distance - radius * 2
    this.camera.far = distance + radius * 2
    this.camera.updateProjectionMatrix()
    const yaw = THREE.MathUtils.degToRad(this.baseYaw() + this.yaw)
    // The camera looks at the folded device's middle: the right half from
    // the hinge to its edge, the left one's edge where the hinge turned it.
    const turned = THREE.MathUtils.degToRad(180 - this.degrees)
    const edge = { x: -radius * Math.cos(turned), z: radius * Math.sin(turned) }
    const center = new THREE.Vector3((Math.min(edge.x, 0) + radius) / 2, 0, edge.z / 2)
    this.camera.position.set(center.x + distance * Math.sin(yaw), 0, center.z + distance * Math.cos(yaw))
    this.camera.lookAt(center)
    // The camera turned counterclockwise shows the device turned clockwise.
    this.camera.rotateZ((this.quarters * Math.PI) / 2)
    this.renderer.render(this.scene, this.camera)
  }

  /**
   * Where `x`, `y` of the canvas, in CSS pixels, falls on the picture; none
   * where it falls on the body first, as on a dark display or the half in
   * front of the lit one.
   */
  displayPoint(x: number, y: number): DisplayPoint | null {
    const pointer = new THREE.Vector2((x / this.size.width) * 2 - 1, -(y / this.size.height) * 2 + 1)
    this.raycaster.setFromCamera(pointer, this.camera)
    const meshes = this.posedMeshes()
    const hit = this.raycaster.intersectObjects([...meshes.keys()], false)[0]
    const mesh = hit && meshes.get(hit.object as THREE.Mesh)
    if (!hit?.uv || !mesh || !this.body.displays[this.shown].includes(mesh)) {
      return null
    }
    // The display's point, turned as the picture is onto it.
    const texture = this.faces[this.shown].texture
    texture.updateMatrix()
    const at = hit.uv.clone().applyMatrix3(texture.matrix)
    return { x: at.x, y: 1 - at.y }
  }

  /** The body's meshes posed at the hinge's angle now, taken again once it moved. */
  private posedMeshes(): Map<THREE.Mesh, THREE.Mesh> {
    if (this.posed?.degrees !== this.degrees) {
      this.posed = { degrees: this.degrees, meshes: posedCopies(this.body.object) }
    }
    // A display wears its picture or its dark finish, whose side a ray respects.
    for (const [copy, mesh] of this.posed.meshes) {
      copy.material = mesh.material
    }
    return this.posed.meshes
  }

  dispose(): void {
    this.disposed = true
    this.body.dispose()
    this.renderer.dispose()
    for (const face of Object.values(this.faces)) {
      face.texture.dispose()
      face.material.dispose()
    }
    this.dark.dispose()
  }
}

/**
 * Each mesh of `object`, with what a ray tests for it: a skinned mesh's
 * plain copy, its vertices where its bones hold them now, in the same place
 * in the world; any other mesh itself.
 */
function posedCopies(object: THREE.Object3D): Map<THREE.Mesh, THREE.Mesh> {
  object.updateMatrixWorld(true)
  const meshes = new Map<THREE.Mesh, THREE.Mesh>()
  const vertex = new THREE.Vector3()
  object.traverse((each) => {
    if (!(each instanceof THREE.Mesh)) {
      return
    }
    if (!(each instanceof THREE.SkinnedMesh)) {
      meshes.set(each, each)
      return
    }
    const source = each.geometry
    const count = source.getAttribute('position').count
    const positions = new Float32Array(count * 3)
    for (let index = 0; index < count; index++) {
      each.getVertexPosition(index, vertex).toArray(positions, index * 3)
    }
    const geometry = new THREE.BufferGeometry()
    geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3))
    geometry.setIndex(source.index)
    const uv = source.getAttribute('uv')
    if (uv) {
      geometry.setAttribute('uv', uv)
    }
    for (const group of source.groups) {
      geometry.addGroup(group.start, group.count, group.materialIndex)
    }
    const copy = new THREE.Mesh(geometry, each.material)
    copy.matrixAutoUpdate = false
    copy.matrixWorld.copy(each.matrixWorld)
    meshes.set(copy, each)
  })
  return meshes
}
