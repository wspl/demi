import type { CatalogProvider } from './assembly'
import type { ControlService } from '../storage/control'
import type { RunnerRegistry } from '../runner/registry'
import { resolveExecutionTarget } from '../conversation/execution-target'

/** Inspects target admission without waking Cloud or starting a provider process. */
export async function executionAvailable(control: ControlService, registry: RunnerRegistry, cloudConfigured: boolean, userId: string, conversationId?: string): Promise<boolean | null> {
  if (!conversationId) return cloudConfigured
  const conversation = await control.getConversation(conversationId)
  if (!conversation || conversation.userId !== userId) return null
  const target = await resolveExecutionTarget(control, registry, conversation)
  if (target.kind === 'cloud') return cloudConfigured
  const device = await control.getDevice(target.deviceId)
  return device?.kind === 'managed' ? cloudConfigured : registry.deviceOnline(target.deviceId)
}

export function modelAvailability(provider: CatalogProvider, execution: boolean) {
  if (provider.auth.status === 'unauthenticated' || provider.auth.status === 'error') return { available: false, reason: 'authentication', message: 'Provider login is unavailable' }
  if (provider.runtime.status === 'unavailable' || provider.runtime.status === 'error') return { available: false, reason: 'runtime', message: provider.runtime.message }
  if (provider.requiresProcessCapableHost && !execution) return { available: false, reason: 'execution', message: 'The selected execution environment is unavailable' }
  return { available: true, reason: null, message: null }
}
