import { onBeforeUnmount, watch, type WatchSource } from 'vue'

/**
 * The browser's Back closes the layer on top of a phone's page, a bottom
 * sheet or a full-screen pane, as Back closes one in a phone's own apps.
 *
 * A layer that opens adds an entry at the same address to the window's
 * history; Back leaves that entry, which closes the layer, and the page's
 * router never hears of it (the `popstate` stops here). A layer that closes
 * another way, a tap or a drag, takes its entry off itself. A navigation the
 * app starts at that moment, such as a sheet's item that opens a
 * conversation, waits for that first (`historySettled`, which the app's
 * router awaits before each navigation): pushed before, the new page would
 * land under the entry about to be taken off.
 *
 * A page shown inside another, as the gallery's phone frames are, adds no
 * entry: the browser's history and its Back belong to the page around it.
 */

/** The key of the layer's token in its entry's history state. */
const LAYER_KEY = 'demiLayer'

/**
 * How many of the `pushed` layers (oldest first) stay open when Back lands on
 * the entry of `landed`: those up to and including it; none when it is
 * none of theirs.
 */
export function layersKeptByBack(pushed: readonly string[], landed: string | null): number {
  return landed === null ? 0 : pushed.indexOf(landed) + 1
}

/** How many entries to take off for layers closed another way: the closed ones at the end of `pushed`. */
export function entriesToTakeOff(pushed: readonly { open: boolean }[]): number {
  let count = 0
  while (count < pushed.length && !pushed[pushed.length - 1 - count]!.open)
    count += 1
  return count
}

interface Layer {
  token: string
  open: boolean
  close: () => void
}

/** The layers with an entry of their own, in the order their entries were added. */
const pushed: Layer[] = []
/** The address the first layer opened at: Back to it is the layers', not the router's. */
let layerAddress = ''
/** The removal of entries under way, which a navigation waits for. */
let takingOff: { done: Promise<void>; finish: () => void } | null = null
let listening = false

function landedToken(state: unknown): string | null {
  if (typeof state !== 'object' || state === null)
    return null
  const token: unknown = Reflect.get(state, LAYER_KEY)
  return typeof token === 'string' ? token : null
}

function onPopState(event: PopStateEvent): void {
  if (takingOff) {
    // The pop is our own removal: the address and the router's entry are as they were.
    event.stopImmediatePropagation()
    takingOff.finish()
    return
  }
  if (pushed.length === 0)
    return
  const closed = pushed.splice(layersKeptByBack(pushed.map((layer) => layer.token), landedToken(event.state)))
  // Back within the layers' entries stays at their address: no navigation for the router.
  if (location.href === layerAddress)
    event.stopImmediatePropagation()
  for (const layer of closed.reverse()) {
    layer.open = false
    layer.close()
  }
}

function listen(): void {
  if (listening)
    return
  listening = true
  // Capturing at the window runs before the router's own listener there, which can then be stopped.
  window.addEventListener('popstate', onPopState, { capture: true })
}

/** Takes off the entries of the layers that closed on top, once the closing is over. */
function takeOffClosed(): void {
  const count = entriesToTakeOff(pushed)
  if (count === 0 || takingOff)
    return
  const top = pushed.at(-1)!
  pushed.splice(pushed.length - count)
  // A navigation already left the layers' entries behind: they stay in the history, harmless.
  if (landedToken(history.state) !== top.token)
    return
  let finish = () => {}
  const done = new Promise<void>((resolve) => {
    finish = resolve
  })
  // The browser answers a traversal with its popstate; should one ever be dropped, navigation must not wait forever.
  const fallback = window.setTimeout(() => takingOff?.finish(), 1000)
  takingOff = {
    done,
    finish: () => {
      window.clearTimeout(fallback)
      takingOff = null
      finish()
    },
  }
  history.go(-count)
}

/** Resolves once no entry of a closed layer is being taken off; the app's router awaits it before navigating. */
export function historySettled(): Promise<void> {
  return takingOff?.done ?? Promise.resolve()
}

/**
 * Gives the layer an entry in the history while `isOpen` (and `enabled`)
 * holds, so that Back closes it through `close`.
 */
export function useBackClose(isOpen: WatchSource<boolean>, close: () => void, enabled: WatchSource<boolean> = () => true): void {
  if (window.top !== window.self)
    return
  let layer: Layer | null = null

  function closed(): void {
    if (!layer)
      return
    layer.open = false
    layer = null
    takeOffClosed()
  }

  watch([isOpen, enabled], ([open, on]) => {
    if (open && on && !layer) {
      listen()
      if (pushed.length === 0)
        layerAddress = location.href
      layer = { token: crypto.randomUUID(), open: true, close: () => {
        layer = null
        close()
      } }
      pushed.push(layer)
      history.pushState({ ...(history.state as object | null), [LAYER_KEY]: layer.token }, '')
    } else if (!(open && on)) {
      closed()
    }
  }, { immediate: true, flush: 'sync' })

  onBeforeUnmount(closed)
}
