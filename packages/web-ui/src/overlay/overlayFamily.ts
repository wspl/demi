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
      if (!panels.includes(el)) panels.push(el)
      return () => {
        const index = panels.indexOf(el)
        if (index >= 0) panels.splice(index, 1)
      }
    },
  }
}

export function isInsideOverlayFamily(family: OverlayFamily, event: Event): boolean {
  const path = event.composedPath()
  return family.panels.some((el) => path.includes(el))
}
