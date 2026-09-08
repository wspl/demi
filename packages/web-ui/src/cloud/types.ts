/** Display state supplied by a Cloud status source. */
export interface CloudState {
  state: 'unallocated' | 'off' | 'booting' | 'running' | 'saving' | 'resetting' | 'unavailable'
  phase: 'stopping' | 'saving' | 'rebuilding' | 'booting' | 'ready' | 'failed' | null
  error: string | null
  systemBytes: number
  homeBytes: number
}
