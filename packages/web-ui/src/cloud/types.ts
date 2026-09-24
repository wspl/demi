/** The two writable filesystems of a Cloud, in bytes: system and home. */
export interface CloudVolumes {
  systemBytes: number
  homeBytes: number
}

/** Display state supplied by a Cloud status source. */
export interface CloudState {
  state: 'unallocated' | 'off' | 'booting' | 'running' | 'saving' | 'resetting'
  /** The operation id of the Cloud's latest reset; null before the first. */
  operationId: string | null
  /** The latest reset's phase; null before the first. */
  phase: 'stopping' | 'saving' | 'rebuilding' | 'booting' | 'ready' | 'failed' | null
  /** Why the Cloud's last boot, save or reset failed. */
  error: string | null
  /** The filesystems' current capacities; null until the Cloud's first start made them. */
  volumes: CloudVolumes | null
  /** The most each filesystem may grow to. */
  limits: CloudVolumes
}
