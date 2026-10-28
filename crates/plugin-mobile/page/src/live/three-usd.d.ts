// The parts of three.js's USD loader the hinged view reads a USDZ's layers
// with, which `@types/three` does not declare.

declare module 'three/addons/loaders/usd/USDCParser.js' {
  /** A parsed USDC layer. */
  export interface UsdLayer {
    specsByPath: Record<string, { fields: Record<string, unknown> }>
  }

  export class USDCParser {
    parseData(buffer: ArrayBuffer): UsdLayer
  }
}

declare module 'three/addons/loaders/usd/USDComposer.js' {
  import type { Group, Texture, Wrapping } from 'three'
  import type { UsdLayer } from 'three/addons/loaders/usd/USDCParser.js'

  export class USDComposer {
    /** The textures the composed scene loads, which it shows once they settle. */
    texturePromises: Promise<unknown>[]
    compose(layer: UsdLayer, assets: Record<string, UsdLayer | Uint8Array>, variants: Record<string, string>, base: string): Group
    /** Makes a texture of an image's bytes; a worker's composer reads them without `Image`. */
    _createTextureFromData(
      data: Uint8Array | ArrayBuffer | string | null,
      textureAttrs: Record<string, unknown> | undefined,
      transformAttrs: Record<string, unknown> | undefined,
    ): Texture | null
    _getWrapMode(mode: unknown): Wrapping
    _applyTextureTransforms(texture: Texture, transformAttrs: Record<string, unknown> | undefined): void
  }
}
