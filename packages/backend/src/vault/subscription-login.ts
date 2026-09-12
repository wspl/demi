import { rename, rm } from 'node:fs/promises'
import { join } from 'node:path'
import type {
  ProviderCredentialLoginOptions,
  ProviderCredentialLoginResult
} from '@demicodes/provider'
import { createId, errorMessage } from '@demicodes/utils'
import type { ProviderAssembly } from '../llm/assembly'
import type { ProviderEntry, ProviderVault } from './providers'
import type { ProviderOperations } from './provider-operations'

export type SubscriptionLoginState =
| {
  status: 'pending';
  verificationUrl: string | null;
  userCode: string | null
}
| {
  status: 'completed';
  providerId: string
}
| {
  status: 'failed';
  message: string
}

interface LoginFlow {
  ownerUserId: string | null
  state: SubscriptionLoginState
  abort: AbortController
  done: Promise<void>
  finishedAt: number | null
}

/**
 * Owns cancellable device logins, their private temporary pools, and
 * publication into the vault.
 */
export class SubscriptionLoginFlows {
  private readonly flows = new Map<string, LoginFlow>()
  private closed = false

  constructor(
    private readonly vault: ProviderVault,
    private readonly assembly: ProviderAssembly,
    private readonly options: {
      vaultRoot: string;
      operations: ProviderOperations
    },
  ) {}

  async start(
    providerType: string,
    label: string,
    ownerUserId: string | null,
    existing?: ProviderEntry
  ): Promise<{ id: string } | { refused: 'no_login_flow' | 'exists' | 'busy' }> {
    if (this.closed)
      return { refused: 'busy' }
    if (providerType === 'claude-code')
      return { refused: 'no_login_flow' }
    this.prune()
    if (!existing && (await this.vault.list({ ownerUserId })).some(
      entry => entry.config.providerType === providerType
    ))
      return { refused: 'exists' }
    const release = existing
      ? this.options.operations.reserve(existing.id)
      : () => {}
    if (!release)
      return { refused: 'busy' }
    const id = createId()
    const pendingDir = existing
      ? null
      : join(this.options.vaultRoot, `pending-${id}`)
    try {
      const provider = existing
        ? (await this.assembly.providerFor(existing.id))?.provider
        : this.assembly.buildDetached(
          providerType,
          { id, label, vaultDir: pendingDir! }
        )
      const begin = provider?.credentials?.beginLogin?.bind(
        provider.credentials
      )
      if (!begin || this.closed) {
        release()
        return { refused: 'no_login_flow' }
      }
      const flow: LoginFlow = {
        ownerUserId,
        state: {
          status: 'pending',
          verificationUrl: null,
          userCode: null
        },
        abort: new AbortController(),
        done: Promise.resolve(),
        finishedAt: null
      }
      this.flows.set(id, flow)
      flow.done = this.complete(flow, begin, { providerType, label, existing, pendingDir })
        .finally(release)
      return { id }
    } catch (error) {
      release()
      throw error
    }
  }

  private async complete(
    flow: LoginFlow,
    begin: (
      options?: ProviderCredentialLoginOptions
    ) => Promise<ProviderCredentialLoginResult>,
    input: {
      providerType: string;
      label: string;
      existing?: ProviderEntry;
      pendingDir: string | null
    }
  ): Promise<void> {
    let unpublishedDir = input.pendingDir
    const timer = setTimeout(
      () => flow.abort.abort(new Error('Login expired')),
      10 * 60_000
    )
    try {
      const result = await begin({
        signal: flow.abort.signal,
        onPending: pending => {
          if (flow.abort.signal.aborted)
            return
          flow.state = {
            status: 'pending',
            verificationUrl: pending.verificationUrl,
            userCode: pending.userCode ?? null
          }
        },
      })
      if (flow.abort.signal.aborted || result.status === 'cancelled')
        throw new Error('Login cancelled or expired')
      if (result.status !== 'completed')
        throw new Error(result.message)
      let providerId = input.existing?.id
      if (!providerId) {
        providerId = createId()
        const destination = this.assembly.vaultDir(providerId)
        await rename(input.pendingDir!, destination)
        unpublishedDir = destination
        await this.vault.create({
          id: providerId,
          ownerUserId: flow.ownerUserId,
          label: input.label,
          config: {
            kind: 'subscription',
            providerType: input.providerType
          }
        })
        unpublishedDir = null
      }
      await this.assembly.invalidate(providerId)
      flow.state = { status: 'completed', providerId }
    } catch (error) {
      flow.state = { status: 'failed', message: errorMessage(error) }
    } finally {
      clearTimeout(timer)
      if (unpublishedDir) {
        try {
          await rm(unpublishedDir, { recursive: true, force: true })
        } catch (error) {
          flow.state = {
            status: 'failed',
            message: `Credential cleanup failed: ${errorMessage(error)}`
          }
        }
      }
      flow.finishedAt = Date.now()
    }
  }

  status(
    id: string,
    ownerUserId: string | null
  ): SubscriptionLoginState | null {
    this.prune()
    const flow = this.flows.get(id)
    return flow && flow.ownerUserId === ownerUserId ? flow.state : null
  }

  async cancel(id: string, ownerUserId: string | null): Promise<boolean> {
    const flow = this.flows.get(id)
    if (!flow || flow.ownerUserId !== ownerUserId)
      return false
    flow.abort.abort()
    await flow.done
    return true
  }

  async close(): Promise<void> {
    this.closed = true
    for (const flow of this.flows.values())
      flow.abort.abort()
    await Promise.all([...this.flows.values()].map(flow => flow.done))
    this.flows.clear()
  }

  private prune(): void {
    for (const [id, flow] of this.flows) {
      if (flow.finishedAt !== null &&
        Date.now() - flow.finishedAt > 10 * 60_000)
        this.flows.delete(id)
    }
  }
}
