import { createStore } from '../store/createStore'

export interface OverlayEntry {
  id: string
  close: () => void
}

export interface OverlayStore {
  state: {
    entries: OverlayEntry[]
  }
  hasEntries(): boolean
  closeTop(): void
  push(id: string, close: () => void): () => void
  remove(id: string): void
  subscribe(listener: () => void): () => void
}

export function createOverlayStore(): OverlayStore {
  const store = createStore<{ entries: OverlayEntry[] }>({ entries: [] })

  return {
    state: store.state,
    subscribe: store.subscribe,
    hasEntries() {
      return store.state.entries.length > 0
    },
    closeTop() {
      const top = store.state.entries[store.state.entries.length - 1]
      if (!top) return
      top.close()
    },
    push(id, close) {
      store.update((state) => {
        state.entries.push({ id, close })
      })

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
