import { reactive } from 'vue'

export interface Store<T extends object> {
  state: T
  patch(partial: Partial<T>): void
  update(mutator: (state: T) => void): void
  subscribe(listener: () => void): () => void
}

export function createStore<T extends object>(initialState: T): Store<T> {
  const state = reactive(initialState) as T
  const listeners = new Set<() => void>()

  function notify() {
    for (const listener of listeners) listener()
  }

  return {
    state,
    patch(partial) {
      Object.assign(state, partial)
      notify()
    },
    update(mutator) {
      mutator(state)
      notify()
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
  }
}
