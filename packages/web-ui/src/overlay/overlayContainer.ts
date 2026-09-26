import { computed, inject, type ComputedRef, type InjectionKey, type Ref } from 'vue'

/**
 * Element floating surfaces teleport into instead of `document.body`. A host that is a
 * containing block (`contain: paint`, a transform) keeps fixed-positioned panels inside
 * itself, so a catalog can pin overlays open without them owning the page.
 */
export const overlayContainerKey: InjectionKey<Ref<HTMLElement | undefined>> =
  Symbol('overlayContainer')

/**
 * When true, a dialog renders in flow as a bare panel: no scrim, no centering, its own
 * size. For a catalog that shows a dialog beside its notes without a page-sized well.
 */
export const overlayInlineKey: InjectionKey<boolean> = Symbol('overlayInline')

/** Where a floating surface renders: its host container, if a catalog provides one, else `body`. */
export interface OverlayTarget {
  /** The host container, or null in the product. */
  container: Ref<HTMLElement | undefined> | null
  /** The element or selector the surface teleports to. */
  to: ComputedRef<HTMLElement | 'body'>
  /**
   * Whether the surface may mount its teleport. A host's container element exists only
   * once the host has mounted, after its content first renders; a teleport that went to
   * `body` first and moved in then would move without its start anchor and put its
   * content after its end anchor, and a dialog holding dialogs of its own would then
   * close against anchors its own unmount removes. So a surface in a host waits for the
   * container, and its teleport never changes target.
   */
  ready: ComputedRef<boolean>
}

/** The target of a floating surface in the component that calls it. */
export function useOverlayTarget(): OverlayTarget {
  const container = inject(overlayContainerKey, null)
  return {
    container,
    to: computed(() => container?.value ?? 'body'),
    ready: computed(() => !container || container.value !== undefined),
  }
}
