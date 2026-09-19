import { createStore } from '../store/createStore'

export interface ThemeStoreState {
  mode: 'light' | 'dark'
}

export interface ThemeStore {
  state: ThemeStoreState
  setMode(mode: ThemeStoreState['mode']): void
  subscribe(listener: () => void): () => void
}

export function createThemeStore(initial: Partial<ThemeStoreState> = {}): ThemeStore {
  const store = createStore<ThemeStoreState>({
    mode: initial.mode ?? 'dark',
  })

  return {
    state: store.state,
    subscribe: store.subscribe,
    setMode(mode) {
      if (store.state.mode === mode)
        return
      store.patch({ mode })
    },
  }
}
