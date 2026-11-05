import type { DeviceReport } from '../devices/report'
import type { Isolation } from '../settings/cloud'

/** The two writable filesystems of a Cloud, in bytes: system and home. */
export interface CloudVolumes {
  systemBytes: number
  homeBytes: number
}

/** A Cloud's move to the isolation `to`: under way, or failed with why. */
export type CloudMoveStatus = { state: 'moving'; to: Isolation } | { state: 'failed'; to: Isolation; message: string }

/** Display state supplied by a Cloud status source. */
export interface CloudState {
  state: 'unallocated' | 'off' | 'booting' | 'running' | 'saving' | 'resetting'
  /** The operation id of the Cloud's latest reset; null before the first. */
  operationId: string | null
  /** The latest reset's phase; null before the first. */
  phase: 'stopping' | 'saving' | 'rebuilding' | 'booting' | 'ready' | 'failed' | null
  /** Why the Cloud's last boot, save or reset failed. */
  error: string | null
  /** The filesystems' capacities; null until the Cloud's first start made them. */
  volumes: CloudVolumes | null
  /** The server runs a newer Cloud image than the Cloud's system is on, which a reset moves it to. */
  newerImage: boolean
  /** The Cloud's move to the isolation the master chose; null while it is on it. */
  moveStatus: CloudMoveStatus | null
  /** The Cloud's device, whose page its row opens; null before its first use made it. */
  deviceId: string | null
  /** What the Cloud's runner last reported; nothing before the Cloud first started. */
  report: DeviceReport
}
