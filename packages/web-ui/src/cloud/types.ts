/** The two writable filesystems of a Cloud, in bytes: system and home. */
export interface CloudVolumes {
  systemBytes: number
  homeBytes: number
}

/** Display state supplied by a Cloud status source. */
export interface CloudState {
  state: 'unallocated' | 'off' | 'booting' | 'running' | 'saving' | 'resetting'
  phase: 'stopping' | 'saving' | 'rebuilding' | 'booting' | 'ready' | 'failed' | null
  error: string | null
  /** The filesystems' current capacities; null until the Cloud's first start made them. */
  volumes: CloudVolumes | null
  /** The most each filesystem may grow to. */
  limits: CloudVolumes
}
