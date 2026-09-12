import type { AgentServer } from '@demicodes/agent'
import type { ProductState as ProductStateResponse } from '@demicodes/product-contracts'
import type { User, InstanceMode } from '../auth/identity'
import type { ControlService } from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'
import type { RunnerRegistry } from '../runner/registry'
import type { ProviderVault } from '../vault/providers'
import type { ProviderAssembly } from '../llm/assembly'
import type { ManagedHosts } from '../managed/lifecycle'
import { providerOwner } from '../vault/scope'
import { publicProvider } from '../vault/public-provider'
import { providerDetails } from '../llm/provider-details'
import { conversationSummary } from '../conversation/summary'
import { errorMessage } from '@demicodes/utils'

/**
 * Re-readable product snapshot. The browser can revalidate this after reconnect
 * without replaying transient events.
 */
export class ProductState {
  constructor(private readonly deps: {
    control: ControlService;
    stores: ConversationStores;
    server: AgentServer;
    registry: RunnerRegistry;
    vault: ProviderVault;
    assembly: ProviderAssembly;
    managed: ManagedHosts | null;
    mode: InstanceMode;
  }) {}

  async read(user: User): Promise<ProductStateResponse> {
    const { control, stores, server, registry, vault, assembly, managed, mode } = this.deps
    const [active, archived, workspaces, devices, preferences, entries, cloud] = await Promise.all(
      [
        control.listConversations(user.id),
        control.listConversations(user.id, { archived: true }),
        control.listWorkspaces(user.id),
        control.listDevices(user.id),
        control.getUserPreferences(user.id),
        vault.list({ ownerUserId: providerOwner(mode, user.id) }),
        managed?.status(user.id) ?? null,
      ]
    )
    const providers = await Promise.all(entries.map(async entry => {
      try {
        const resolved = await assembly.providerFor(entry.id)
        return {
          ...publicProvider(entry),
          details: resolved ? await providerDetails(
            resolved.provider,
            entry.config.kind === 'subscription'
          ) : null
        }
      } catch (error) {
        return {
          ...publicProvider(entry),
          details: null,
          error: errorMessage(error)
        }
      }
    }))
    return {
      user,
      mode,
      preferences,
      workspaces,
      providers,
      conversations: [...active, ...archived].map(
        conversation => conversationSummary(conversation, stores, server)
      ),
      devices: devices.map(device => ({
        ...device,
        online: registry.deviceOnline(device.id),
        home: registry.deviceIdentity(device.id)?.homeDir ?? null
      })),
      cloud: cloud ? {
        ...cloud,
        device: cloud.device
          ? { id: cloud.device.id, name: cloud.device.name }
          : null
      } : null,
    }
  }
}
