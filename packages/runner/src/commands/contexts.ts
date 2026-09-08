import type { Manifest } from '@demicodes/command-loader'
import { LOCAL } from '@demicodes/runner-protocol/local'

export interface ExecutionContext {
  id: string
  owner: string
  jobId?: string
  agentSessionId: string
  shellId: string
  manifest: Manifest | null
}

/** Only live processes created by this runner receive an execution context. */
export class ExecutionContexts {
  private readonly contexts = new Map<string, ExecutionContext>()
  create(owner: string, env: Record<string, string>, manifest: Manifest | null, jobId?: string): ExecutionContext {
    const context: ExecutionContext = { id: crypto.randomUUID().replaceAll('-', ''), owner, ...(jobId ? { jobId } : {}), agentSessionId: jobId ? env.DEMI_SESSION_ID ?? '' : '', shellId: jobId ? env.DEMI_SHELL_ID ?? '' : '', manifest }
    this.contexts.set(context.id, context)
    return context
  }
  get(id: string): ExecutionContext {
    const context = this.contexts.get(id)
    if (!context) throw new Error('execution context is not live on this runner')
    return context
  }
  remove(owner: string): void {
    for (const [id, context] of this.contexts) if (context.owner === owner) this.contexts.delete(id)
  }
  get count(): number { return this.contexts.size }
  clear(): void { this.contexts.clear() }
  environment(context: ExecutionContext, endpoint: string): Record<string, string> {
    return { [LOCAL.endpointEnv]: endpoint, [LOCAL.contextEnv]: context.id }
  }
}
