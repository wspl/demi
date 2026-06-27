export interface StoreStateSource<T extends object> {
  state: T
}

export function useStore<T extends object>(store: StoreStateSource<T>): T {
  return store.state
}
