// The machine manager's wire declared as zod schemas: the single source of
// truth for both directions (`ManagedHostProvisioner` is what the calls
// mean; this is how they travel). Frames are newline-delimited JSON over a
// Unix socket: one request per line, one `ok`/`error` per request, `death`
// on the manager's own initiative. Ids are opaque strings the client picks.
import { z } from 'zod'
import { imageStateSchema, type BootArgs } from './provisioner'

const deviceId = z.string().min(1)

const bootArgsSchema: z.ZodType<BootArgs> = z.object({
  backendUrl: z.string().min(1),
  deviceToken: z.string().min(1),
})

/**
 * One entry per `ManagedHostProvisioner` method: its parameters and its
 * result, from which the request union and the typed replies derive.
 */
export const machineOps = {
  reconcile: { params: z.object({}), result: z.null() },
  current_base_version: { params: z.object({}), result: z.string() },
  image_state: {
    params: z.object({ deviceId }),
    result: imageStateSchema.nullable()
  },
  wake: { params: z.object({ deviceId, boot: bootArgsSchema }), result: z.null() },
  hibernate: { params: z.object({ deviceId }), result: z.null() },
  checkpoint: { params: z.object({ deviceId }), result: z.null() },
  grow_volume: {
    params: z.object({
      deviceId,
      volume: z.enum(['system', 'home']),
      bytes: z.number().int().positive()
    }),
    result: z.null()
  },
  reset: {
    params: z.object({
      deviceId,
      operationId: z.string().min(1),
      baseVersion: z.string().min(1)
    }),
    result: z.null()
  },
} as const

export type MachineOp = keyof typeof machineOps
export type MachineParams<Op extends MachineOp> =
  z.infer<(typeof machineOps)[Op]['params']>
export type MachineResult<Op extends MachineOp> =
  z.infer<(typeof machineOps)[Op]['result']>

const id = z.string().min(1)

export const machineRequestSchema = z.discriminatedUnion('op', [
  z.object({ id, op: z.literal('reconcile'), params: machineOps.reconcile.params }),
  z.object({ id, op: z.literal('current_base_version'), params: machineOps.current_base_version.params }),
  z.object({ id, op: z.literal('image_state'), params: machineOps.image_state.params }),
  z.object({ id, op: z.literal('wake'), params: machineOps.wake.params }),
  z.object({ id, op: z.literal('hibernate'), params: machineOps.hibernate.params }),
  z.object({ id, op: z.literal('checkpoint'), params: machineOps.checkpoint.params }),
  z.object({ id, op: z.literal('grow_volume'), params: machineOps.grow_volume.params }),
  z.object({ id, op: z.literal('reset'), params: machineOps.reset.params }),
])
export type MachineRequest = z.infer<typeof machineRequestSchema>

/**
 * A reply names the request; `result` is validated by the op's own schema
 * once the client knows which op it answers. `death` is unsolicited.
 */
export const machineResponseSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('ok'), id, result: z.unknown() }),
  z.object({ type: z.literal('error'), id, message: z.string() }),
  z.object({ type: z.literal('death'), deviceId }),
])
export type MachineResponse = z.infer<typeof machineResponseSchema>
