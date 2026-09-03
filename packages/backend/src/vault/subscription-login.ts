import { rename, rm } from 'node:fs/promises'
import { join } from 'node:path'
import type { ProviderCredentialLoginOptions, ProviderCredentialLoginResult } from '@demicodes/provider'
import { createId, errorMessage } from '@demicodes/utils'
import type { ProviderAssembly } from '../llm/assembly'
import type { ProviderVault } from './providers'

export type SubscriptionLoginState =
  | { status: 'pending'; verificationUrl: string | null; userCode: string | null }
  | { status: 'completed'; providerId: string }
  | { status: 'failed'; message: string }

interface LoginFlow {
  providerType: string
  label: string
  ownerUserId: string | null
  pendingDir: string
  state: SubscriptionLoginState
}

/**
 * Subscription device-login flows: the provider runs its vendor's public
 * protocol natively against a throwaway credential pool; the web UI shows
 * the pending verification URL/code and polls. On completion the pool
 * directory becomes the new provider's vault directory and the provider
 * row is created — nothing token-shaped ever crosses the HTTP surface.
 */
export class SubscriptionLoginFlows {
  private readonly flows = new Map<string, LoginFlow>()

  constructor(
    private readonly vault: ProviderVault,
    private readonly assembly: ProviderAssembly,
    private readonly options: { vaultRoot: string },
  ) {}

  /** Starts one login for the provider scope's owner; returns its poll id, or null for a type without a native login flow. */
  start(providerType: string, label: string, ownerUserId: string | null): { id: string } | null {
    const id = createId()
    const pendingDir = join(this.options.vaultRoot, `pending-${id}`)
    let begin: ((options?: ProviderCredentialLoginOptions) => Promise<ProviderCredentialLoginResult>) | undefined
    try {
      // API-key types refuse subscription configs at construction — same answer: no login flow.
      const provider = this.assembly.buildDetached(providerType, { id, label, vaultDir: pendingDir })
      begin = provider.credentials?.beginLogin?.bind(provider.credentials)
    } catch {
      return null
    }
    if (!begin) return null

    const flow: LoginFlow = { providerType, label, ownerUserId, pendingDir, state: { status: 'pending', verificationUrl: null, userCode: null } }
    this.flows.set(id, flow)
    void (async () => {
      try {
        const result = await begin({
          onPending: (pending) => {
            if (flow.state.status !== 'pending') return
            flow.state = {
              status: 'pending',
              verificationUrl: pending.verificationUrl,
              userCode: pending.userCode ?? null,
            }
          },
        })
        if (result.status !== 'completed') {
          flow.state = { status: 'failed', message: result.status === 'cancelled' ? 'Login cancelled' : result.message }
          await rm(pendingDir, { recursive: true, force: true })
          return
        }
        const provider = await this.vault.create({
          ownerUserId: flow.ownerUserId,
          label: flow.label,
          config: { kind: 'subscription', providerType: flow.providerType },
        })
        await rename(pendingDir, this.assembly.vaultDir(provider.id))
        flow.state = { status: 'completed', providerId: provider.id }
      } catch (error) {
        flow.state = { status: 'failed', message: errorMessage(error) }
        await rm(pendingDir, { recursive: true, force: true }).catch(() => {})
      }
    })()
    return { id }
  }

  /** A flow is visible to the scope that started it. */
  status(id: string, ownerUserId: string | null): SubscriptionLoginState | null {
    const flow = this.flows.get(id)
    return flow && flow.ownerUserId === ownerUserId ? flow.state : null
  }
}
