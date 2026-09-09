import { createStore } from '../store/createStore'

/**
 * Exclusive layers (menu, popover, dialog) dismiss hints and replace each other.
 * A stacked layer is a dialog opened from inside another: it dismisses hints too,
 * but keeps what it stands on, and goes when that closes. Hints never cover either.
 */
export type OverlayLayer = 'exclusive' | 'stacked' | 'hint'

export interface OverlayEntry {
  id: string
  layer: OverlayLayer
  close: () => void
  /** The exclusive entry a stacked one stands on. */
  parent?: string
}

export interface OverlayStore {
  state: {
    entries: OverlayEntry[]
  }
  hasEntries(): boolean
  hasExclusive(): boolean
  /** Whether this entry is the one Escape and the scrim address. */
  isTop(id: string): boolean
  closeTop(): void
  push(id: string, close: () => void, layer?: OverlayLayer): () => void
  remove(id: string): void
  subscribe(listener: () => void): () => void
}

function isExclusive(entry: OverlayEntry): boolean {
  return entry.layer !== 'hint'
}

export function createOverlayStore(): OverlayStore {
  const store = createStore<{ entries: OverlayEntry[] }>({ entries: [] })

  function exclusives(): OverlayEntry[] {
    return store.state.entries.filter(isExclusive)
  }

  function dismissHints(): void {
    const hints = store.state.entries.filter((entry) => entry.layer === 'hint')
    if (hints.length === 0)
      return
    store.update((state) => {
      state.entries = state.entries.filter(isExclusive)
    })
    for (const hint of hints) hint.close()
  }

  /** Drops an entry and closes whatever was stacked on it. */
  function drop(id: string): void {
    const index = store.state.entries.findIndex((entry) => entry.id === id)
    if (index < 0)
      return
    store.update((state) => {
      state.entries.splice(index, 1)
    })
    for (const child of store.state.entries.filter((entry) => entry.parent === id)) {
      drop(child.id)
      child.close()
    }
  }

  return {
    state: store.state,
    subscribe: store.subscribe,
    isTop(id) {
      return exclusives().at(-1)?.id === id
    },
    hasExclusive() {
      return exclusives().length > 0
    },
    hasEntries() {
      return exclusives().length > 0
    },
    closeTop() {
      const top = exclusives().at(-1)
      if (!top)
        return
      top.close()
    },
    push(id, close, layer = 'exclusive') {
      if (layer === 'hint' && exclusives().length > 0) {
        return () => {}
      }

      dismissHints()
      const previous = layer === 'exclusive' ? exclusives() : []
      const parent = layer === 'stacked' ? exclusives().at(-1)?.id : undefined

      store.update((state) => {
        state.entries.push({ id, layer, close, parent })
      })

      for (const entry of previous) {
        drop(entry.id)
        entry.close()
      }

      return () => drop(id)
    },
    remove(id) {
      drop(id)
    },
  }
}
