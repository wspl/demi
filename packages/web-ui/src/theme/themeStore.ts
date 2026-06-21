import { createStore } from '../store/createStore'

export interface ThemeStoreState {
  mode: 'light' | 'dark'
  codeThemeId: string
}

export interface ThemeStore {
  state: ThemeStoreState
  setMode(mode: ThemeStoreState['mode']): void
  setCodeThemeId(codeThemeId: string): void
  subscribe(listener: () => void): () => void
}

export function createThemeStore(initial: Partial<ThemeStoreState> = {}): ThemeStore {
  const store = createStore<ThemeStoreState>({
    mode: initial.mode ?? 'dark',
    codeThemeId: initial.codeThemeId ?? 'one',
  })

  return {
    state: store.state,
    subscribe: store.subscribe,
    setMode(mode) {
      if (store.state.mode === mode) return
      store.patch({ mode })
    },
    setCodeThemeId(codeThemeId) {
      if (store.state.codeThemeId === codeThemeId) return
      store.patch({ codeThemeId })
    },
  }
}
