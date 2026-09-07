import type { InjectionKey, Ref } from 'vue'

/**
 * Element floating surfaces teleport into instead of `document.body`. A host that is a
 * containing block (`contain: paint`, a transform) keeps fixed-positioned panels inside
 * itself, so a catalog can pin overlays open without them owning the page.
 */
export const overlayContainerKey: InjectionKey<Ref<HTMLElement | undefined>> = Symbol('overlayContainer')

/**
 * When true, a dialog renders in flow as a bare panel: no scrim, no centering, its own
 * size. For a catalog that shows a dialog beside its notes without a page-sized well.
 */
export const overlayInlineKey: InjectionKey<boolean> = Symbol('overlayInline')
