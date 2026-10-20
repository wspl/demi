import { inject, provide, type InjectionKey } from 'vue'

/** The z-index of the page's first floating layer: a menu over the page, or a dialog. */
export const LAYER_Z = 50

/** The z-index of the toasts' layer, above a menu or a dialog, whose own tips stand above it. */
export const TOAST_Z = 55

const stackingKey: InjectionKey<() => number> = Symbol('stacking')

/**
 * Where a floating surface that opens here stands: one above the layer it opens
 * from, so a menu in a dialog stacked over another dialog shows over both, and a
 * submenu over its menu.
 *
 * Whatever opens from inside the surface stands above it in turn.
 */
export function stackAbove(): () => number {
  const z = showAbove()
  provide(stackingKey, z)
  return z
}

/**
 * Where a surface that only shows stands (a tooltip, a hover card): above the layer
 * it shows over, a hint inside a dialog above the dialog and a hint on a menu's row
 * above the menu.
 *
 * Unlike `stackAbove`, it raises nothing inside itself, since nothing opens from a
 * hint, though a tip's trigger sits in its component and may open a layer of its
 * own.
 */
export function showAbove(): () => number {
  const below = inject(stackingKey, () => LAYER_Z - 1)
  return () => below() + 1
}

/** Makes `z` the z-index the surfaces that open inside this layer stand above. */
export function provideStacking(z: () => number): void {
  provide(stackingKey, z)
}
