import type { InjectionKey, Ref } from 'vue'

/**
 * Element floating surfaces teleport into instead of `document.body`. A host that is a
 * containing block (`contain: paint`, a transform) keeps fixed-positioned panels inside
 * itself, so a catalog can pin overlays open without them owning the page.
 */
export const overlayContainerKey: InjectionKey<Ref<HTMLElement | undefined>> = Symbol('overlayContainer')
