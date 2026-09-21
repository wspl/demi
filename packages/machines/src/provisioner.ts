import { z } from 'zod'
import type { ManagedBoot } from '@demicodes/runner-protocol'

export type ManagedVolume = 'system' | 'home'
export const runtimeStateSchema = z.enum(['running', 'stopped'])
export type MachineRuntimeState = z.infer<typeof runtimeStateSchema>

export type BootArgs = ManagedBoot

export const imageStateSchema = z.object({
  generation: z.string().regex(/^[A-Za-z0-9_-]+$/),
  baseVersion: z.string().regex(/^[A-Za-z0-9_-]+$/),
  resetId: z.string().nullable(),
  systemBytes: z.number().int().positive(),
  homeBytes: z.number().int().positive(),
}).strict()
export type MachineImageState = z.infer<typeof imageStateSchema>

/**
 * Machine operations are serialized by device. Disks never depend on a
 * conversation.
 */
export interface ManagedHostProvisioner {
  reconcile(): Promise<void>
  currentBaseVersion(): Promise<string>
  imageState(deviceId: string): Promise<MachineImageState | null>
  runtimeState(deviceId: string): Promise<MachineRuntimeState>
  /**
   * Creates initial disks on first use, otherwise boots the committed
   * generation.
   */
  wake(deviceId: string, boot: BootArgs): Promise<void>
  hibernate(deviceId: string): Promise<void>
  checkpoint(deviceId: string): Promise<void>
  growVolume(
    deviceId: string,
    volume: ManagedVolume,
    bytes: number
  ): Promise<void>
  /**
   * Idempotent by operation id, even after a backend restart. Does not boot.
   */
  reset(
    deviceId: string,
    operationId: string,
    baseVersion: string
  ): Promise<void>
  close(): Promise<void>
  onDeath(listener: (deviceId: string) => void): void
}
