import { errorCode, noop } from '@demicodes/utils'
import type { ShellEnvironment } from '@demicodes/shell'
import type { SessionNode } from '../node/node'
import type { AgentSession } from '../session/session'
import type { ChildSupervisor } from '../subagent/supervisor'
import type { ServerFrame } from '../protocol/frames'
import type { AgentToolInvokeContext, SessionEvent } from '../types'
import { errorDiagnostics, progressToOutput, progressToShellOutput } from './summaries'

/**
 * The root node as a transport sees it, owned by the server (via the
 * ownership registry), not by any transport binding: the node itself plus
 * the frame sink — the currently attached binding, or nowhere while detached
 * (turns keep running either way) — and the provider the client named.
 */
export class LiveSession {
  readonly node: SessionNode<unknown>
  providerId: string
  sink: (frame: ServerFrame) => void = noop

  private readonly unsubscribeSession: () => void

  constructor(node: SessionNode<unknown>, providerId: string) {
    this.node = node
    this.providerId = providerId
    this.unsubscribeSession = node.session.subscribe((event) => this.handleSessionEvent(event))
  }

  get agentSessionId(): string {
    return this.node.id
  }

  get session(): AgentSession<unknown> {
    return this.node.session
  }

  get supervisor(): ChildSupervisor<unknown> {
    return this.node.supervisor
  }

  get cwd(): string {
    return this.node.cwd
  }

  attachSink(sink: (frame: ServerFrame) => void): void {
    this.sink = sink
  }

  detachSink(): void {
    this.sink = noop
  }

  async dispose(): Promise<void> {
    try {
      await this.node.supervisor.dispose()
      await this.node.session.dispose()
      await this.node.disposeEnvironments()
      await this.node.agent.dispose?.({
        agentSessionId: this.node.id,
        state: this.node.session.state(),
        cwd: this.node.cwd,
        transcript: this.node.session.transcript(),
      })
    } finally {
      this.unsubscribeSession()
      this.detachSink()
    }
  }

  hasShell(shellId: string): boolean {
    return this.node.hasShell(shellId)
  }

  resolveEnvironment(
    ctx: Pick<AgentToolInvokeContext<unknown>, 'state' | 'metadata'>,
    handle: { shellId?: string; commandId?: string },
  ): Promise<ShellEnvironment> {
    return this.node.resolveEnvironment(ctx, handle)
  }

  private handleSessionEvent(event: SessionEvent): void {
    switch (event.type) {
      case 'transcript_changed':
        this.sink({ type: 'transcript_patch', patches: event.patches, revision: event.revision })
        return
      case 'phase_changed':
        this.sink({ type: 'phase', phase: event.phase })
        return
      case 'queue_changed':
        this.sink({ type: 'queue', queue: event.queue })
        return
      case 'tool_progress': {
        this.emitToolProgress(event.toolCallId, event.toolName, event.progress)
        return
      }
      case 'retry_scheduled':
        this.sink({
          type: 'retry_scheduled',
          attempt: event.attempt,
          delayMs: event.delayMs,
          code: event.code,
          diagnostics: event.diagnostics,
        })
        return
      case 'error': {
        const normalized = event.error instanceof Error ? event.error : new Error(String(event.error))
        const code = errorCode(event.error)
        const diagnostics = errorDiagnostics(event.error)
        this.sink({
          type: 'error',
          message: normalized.message,
          ...(code ? { code } : {}),
          ...(diagnostics ? { diagnostics } : {}),
        })
        return
      }
    }
  }

  private emitToolProgress(toolCallId: string, toolName: string, progress: unknown): void {
    const output = progressToOutput(progress)
    this.sink({ type: 'tool_progress', toolUseId: toolCallId, output })
    const shell = toolName === 'shell_status' ? null : progressToShellOutput(progress)
    if (shell) {
      this.sink({
        type: 'shell_output',
        shellId: shell.shellId,
        commandId: shell.commandId,
        status: shell.status,
      })
    }
  }
}
