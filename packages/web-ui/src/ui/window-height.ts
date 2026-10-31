import { readonly, ref } from 'vue'

/**
 * The window's inner height, for whatever sizes itself by it: read once, and
 * kept by one listener of the window's resize for the whole page.
 *
 * Reading `window.innerHeight` lays the page out first whenever anything
 * changed since its last layout, so a component reading it as it mounts, in
 * a transcript that mounts many, lays the page out once for each.
 *
 * Where there is no window, as where the module is only imported, it is 0.
 */
const height = ref(typeof window === 'undefined' ? 0 : window.innerHeight)
if (typeof window !== 'undefined') {
  addEventListener('resize', () => {
    height.value = window.innerHeight
  })
}

export const windowHeight = readonly(height)
