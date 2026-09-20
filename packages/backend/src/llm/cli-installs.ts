import { errorMessage } from '@demicodes/utils'
import type { ProviderEntry } from '../vault/providers'
import type { SessionProviderContext } from './assembly'

/** What the last install of an entry's CLI on the user's Cloud came to. */
export type CliInstallState =
  | { state: 'installing' }
  | { state: 'installed'; path: string }
  | { state: 'failed'; message: string }

/**
 * Installs of a process provider's CLI that no conversation asked for
 * (`claude-cli.md` § Where it runs): after an account is added, and when the
 * user asks again. They run in the background; their outcome is kept for the
 * settings page and never undoes the account.
 */
export class CliInstalls {
  private readonly states = new Map<string, CliInstallState>()
  private readonly running = new Set<Promise<void>>()
  private closed = false

  constructor(private readonly place: (
    userId: string,
    entry: ProviderEntry
  ) => Promise<{
    session: SessionProviderContext
    release(): void
  }>) {}

  /** Starts an install unless one for this entry is already under way. */
  start(userId: string, entry: ProviderEntry): void {
    const key = `${userId}\0${entry.id}`
    if (this.closed || this.states.get(key)?.state === 'installing')
      return
    this.states.set(key, { state: 'installing' })
    const run = (async () => {
      try {
        const placed = await this.place(userId, entry)
        try {
          this.states.set(key, {
            state: 'installed',
            path: (await placed.session.claudeProcess()).command
          })
        } finally {
          placed.release()
        }
      } catch (error) {
        this.states.set(key, { state: 'failed', message: errorMessage(error) })
      }
    })()
    this.running.add(run)
    void run.finally(() => this.running.delete(run))
  }

  state(userId: string, providerId: string): CliInstallState | null {
    return this.states.get(`${userId}\0${providerId}`) ?? null
  }

  forget(providerId: string): void {
    for (const key of this.states.keys()) {
      if (key.endsWith(`\0${providerId}`))
        this.states.delete(key)
    }
  }

  async close(): Promise<void> {
    this.closed = true
    await Promise.all(this.running)
  }
}
