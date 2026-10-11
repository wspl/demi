import { inject, provide, type InjectionKey } from 'vue'

/** The z-index of the page's first floating layer: a menu over the page, or a dialog. */
export const LAYER_Z = 50

const stackingKey: InjectionKey<() => number> = Symbol('stacking')

/**
 * Where a floating surface that opens here stands: one above the layer it
 * opens from, so a menu in a dialog stacked over another dialog shows over
 * both, and a submenu over its menu. Whatever opens from inside the surface
 * stands above it in turn.
 */
export function stackAbove(): () => number {
  const below = inject(stackingKey, () => LAYER_Z - 1)
  const z = () => below() + 1
  provide(stackingKey, z)
  return z
}

/** Makes `z` the z-index the surfaces that open inside this layer stand above. */
export function provideStacking(z: () => number): void {
  provide(stackingKey, z)
}
