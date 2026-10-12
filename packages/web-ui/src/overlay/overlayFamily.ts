import type { InjectionKey } from 'vue'

/** Floating panels that belong to one exclusive tree (root menu + flyouts). */
export interface OverlayFamily {
  panels: HTMLElement[]
  register(el: HTMLElement): () => void
}

export const overlayFamilyKey: InjectionKey<OverlayFamily> = Symbol('overlayFamily')

export function createOverlayFamily(): OverlayFamily {
  const panels: HTMLElement[] = []
  return {
    panels,
    register(el) {
      if (!panels.includes(el))
        panels.push(el)
      return () => {
        const index = panels.indexOf(el)
        if (index >= 0)
          panels.splice(index, 1)
      }
    },
  }
}

/**
 * The overlay-store id of the layer a family belongs to (the dialog, sheet or
 * root menu that provides it): a menu or popover opened inside registers under
 * it as a nested layer.
 */
export const overlayOwnerKey: InjectionKey<string> = Symbol('overlayOwner')
