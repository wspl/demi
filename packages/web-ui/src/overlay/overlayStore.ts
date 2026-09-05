import { createStore } from '../store/createStore'

/** Exclusive layers (menu, popover, dialog) dismiss hints. Hints never cover them. */
export type OverlayLayer = 'exclusive' | 'hint'

export interface OverlayEntry {
  id: string
  layer: OverlayLayer
  close: () => void
}

export interface OverlayStore {
  state: {
    entries: OverlayEntry[]
  }
  hasEntries(): boolean
  hasExclusive(): boolean
  closeTop(): void
  push(id: string, close: () => void, layer?: OverlayLayer): () => void
  remove(id: string): void
  subscribe(listener: () => void): () => void
}

function isExclusive(entry: OverlayEntry): boolean {
  return entry.layer === 'exclusive'
}

export function createOverlayStore(): OverlayStore {
  const store = createStore<{ entries: OverlayEntry[] }>({ entries: [] })

  function exclusives(): OverlayEntry[] {
    return store.state.entries.filter(isExclusive)
  }

  function dismissHints(): void {
    const hints = store.state.entries.filter((entry) => entry.layer === 'hint')
    if (hints.length === 0) return
    store.update((state) => {
      state.entries = state.entries.filter(isExclusive)
    })
    for (const hint of hints) hint.close()
  }

  return {
    state: store.state,
    subscribe: store.subscribe,
    hasExclusive() {
      return exclusives().length > 0
    },
    hasEntries() {
      return exclusives().length > 0
    },
    closeTop() {
      const top = exclusives().at(-1)
      if (!top) return
      top.close()
    },
    push(id, close, layer = 'exclusive') {
      if (layer === 'hint' && exclusives().length > 0) {
        return () => {}
      }

      let previous: OverlayEntry[] = []
      if (layer === 'exclusive') {
        dismissHints()
        previous = exclusives()
      }

      store.update((state) => {
        state.entries.push({ id, layer, close })
      })

      for (const entry of previous) {
        entry.close()
        const index = store.state.entries.findIndex((item) => item.id === entry.id)
        if (index >= 0) {
          store.update((state) => {
            state.entries.splice(index, 1)
          })
        }
      }

      return () => {
        const index = store.state.entries.findIndex((entry) => entry.id === id)
        if (index < 0) return
        store.update((state) => {
          state.entries.splice(index, 1)
        })
      }
    },
    remove(id) {
      const index = store.state.entries.findIndex((entry) => entry.id === id)
      if (index < 0) return
      store.update((state) => {
        state.entries.splice(index, 1)
      })
    },
  }
}
