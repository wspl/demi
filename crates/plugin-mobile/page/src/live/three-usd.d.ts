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
  import type { Group } from 'three'
  import type { UsdLayer } from 'three/addons/loaders/usd/USDCParser.js'

  export class USDComposer {
    /** The textures the composed scene loads, which it shows once they settle. */
    texturePromises: Promise<unknown>[]
    compose(layer: UsdLayer, assets: Record<string, UsdLayer | Uint8Array>, variants: Record<string, string>, base: string): Group
  }
}
