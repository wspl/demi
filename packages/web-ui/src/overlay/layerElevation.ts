import { inject, provide, type InjectionKey } from 'vue'

/** The elevation of the nearest floating layer around a component; the page is 0. */
const layerElevationKey: InjectionKey<number> = Symbol('layerElevation')

/**
 * The elevation of a floating surface opened here: one above the layer it opens from, so
 * a menu over the page is 1, a menu in a dialog 2 and its submenu 3. The surface puts it
 * on itself as `--elevation`, and the theme lifts its fill one step per layer beneath it
 * (`base.css`, floating layers). A surface that only shows (a tooltip, a hover card)
 * reads its elevation with this; a layer that other surfaces open from (a menu, a
 * dialog) uses `provideLayerElevation`.
 */
export function useLayerElevation(): number {
  return inject(layerElevationKey, 0) + 1
}

/** Like `useLayerElevation`, and whatever opens from inside this layer floats above it. */
export function provideLayerElevation(): number {
  const elevation = useLayerElevation()
  provide(layerElevationKey, elevation)
  return elevation
}
