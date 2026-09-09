import { z } from 'zod'
import { shellQuote } from '@demicodes/shell'
import type {
  ControlService,
  ConversationRecord,
  DeviceRecord,
} from '../storage/control'
import type { RunnerRegistry } from '../runner/registry'
import { resolveExecutionTarget } from './execution-target'

export const remoteFileRefSchema = z.strictObject({
  type: z.literal('remote_file'),
  deviceId: z.string().min(1),
  path: z
    .string()
    .min(1)
    .max(4096)
    .refine(
      (path) => path.startsWith('/') && !path.includes('\0'),
      'Expected an absolute path',
    ),
})

/**
 * References retain device identity and read execution-time bytes through the
 * existing host command.
 */
export async function resolveRemoteFileRefs(
  control: ControlService,
  registry: Pick<RunnerRegistry, 'deviceOnline' | 'deviceIdentity'>,
  conversation: ConversationRecord,
  content: unknown[],
): Promise<unknown[]> {
  const references = content
    .map((block) => remoteFileRefSchema.safeParse(block))
    .filter((parsed) => parsed.success)
    .map((parsed) => parsed.data)
  if (references.length === 0) {
    return content
  }
  const devices = new Map<string, DeviceRecord>()
  for (const ref of references) {
    const device = await control.getDevice(ref.deviceId)
    if (!device || device.userId !== conversation.userId) {
      throw new Error('Referenced device is not accessible')
    }
    if (!registry.deviceOnline(device.id)) {
      throw new Error(`Referenced device ${device.name} is offline`)
    }
    devices.set(device.id, device)
  }
  const target = await resolveExecutionTarget(control, registry, conversation)
  for (const device of devices.values()) {
    if (device.id !== target.deviceId) {
      await control.attachHost(conversation.id, device.id, device.name, null, true)
    }
  }
  return content.map((block) => {
    const parsed = remoteFileRefSchema.safeParse(block)
    if (!parsed.success) {
      return block
    }
    const ref = parsed.data
    const device = devices.get(ref.deviceId)!
    const command = `demi host shell --host ${shellQuote(ref.deviceId)} ${shellQuote(`cat -- ${shellQuote(ref.path)}`)}`
    const reference = new URL('file:///')
    reference.pathname = ref.path
    reference.searchParams.set('host', device.name)
    reference.searchParams.set('deviceId', device.id)
    reference.searchParams.set('readCommand', command)
    return {
      type: 'reference',
      reference: reference.href,
    }
  })
}
