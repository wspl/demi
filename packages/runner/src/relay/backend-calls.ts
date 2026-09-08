import type { RpcInvocation } from '@demicodes/command-loader'
import type { PipeRef, RunnerToBackendMessage, BackendToRunnerMessage } from '@demicodes/runner-protocol'
import type { PipeEnds } from '../pipes'
import type { ExecutionContext } from '../commands/contexts'
import { createId, noop, SerialQueue } from '@demicodes/utils'

interface Call {
  invocation: RpcInvocation
  resolve(value: { exitCode: number }): void
  reject(error: unknown): void
  writes: SerialQueue
  stdout: Promise<void>
}
/** Backend RPC is a runner responsibility, independent of the local client's wire. */
export class BackendCalls {
  private readonly calls = new Map<string, Call>()
  constructor(private readonly send: (message: RunnerToBackendMessage) => void, private readonly pipes: PipeEnds) {}
  async invoke(context: ExecutionContext, invocation: RpcInvocation): Promise<{ exitCode: number }> {
    if (!context.jobId || !context.agentSessionId || !context.shellId) throw new Error('rpc requires a backend-dispatched job and session')
    const callId = createId()
    const result = new Promise<{ exitCode: number }>((resolve, reject) => this.calls.set(callId, { invocation, resolve, reject, writes: new SerialQueue(), stdout: Promise.resolve() }))
    result.catch(noop)
    const abort = () => {
      this.calls.get(callId)?.reject(invocation.signal.reason ?? new Error('command cancelled'))
      this.calls.delete(callId)
      try { this.send({ type: 'rpc_cancel', callId }) } catch { /* Already disconnected. */ }
    }
    invocation.signal.addEventListener('abort', abort, { once: true })
    try {
      if (invocation.signal.aborted) { abort(); return await result }
      this.send({ type: 'rpc_call', callId, jobId: context.jobId, agentSessionId: context.agentSessionId, shellId: context.shellId, root: invocation.root, path: invocation.path, argv: invocation.argv, args: invocation.args, json: invocation.json, cwd: invocation.cwd, env: invocation.env, stdin: invocation.stdin !== null })
      void (async () => {
        for await (const bytes of invocation.stdinStream) {
          if (!this.calls.has(callId)) return
          this.send({ type: 'rpc_stdin', callId, bytes })
        }
        if (this.calls.has(callId)) this.send({ type: 'rpc_stdin_end', callId })
      })().catch(error => this.calls.get(callId)?.reject(error))
      return await result
    } finally {
      invocation.signal.removeEventListener('abort', abort)
      this.calls.delete(callId)
    }
  }
  handleReply(message: Extract<BackendToRunnerMessage, { type: 'rpc_pipes' | 'rpc_output' | 'rpc_exit' }>): void {
    const call = this.calls.get(message.callId)
    if (!call) return
    if (message.type === 'rpc_pipes') {
      if (message.stdin) void this.upload(call, message.stdin).catch(call.reject)
      call.stdout = this.download(call, message.stdout)
      void call.stdout.catch(call.reject)
    } else if (message.type === 'rpc_output') {
      void call.writes.run(() => Promise.resolve(call.invocation.io.stderr(message.bytes))).catch(call.reject)
    } else {
      void call.stdout.then(() => call.writes.run(async () => call.resolve({ exitCode: message.exitCode }))).catch(call.reject)
    }
  }
  close(): void {
    for (const call of this.calls.values()) call.reject(new Error('backend disconnected'))
    this.calls.clear()
  }
  private report(ref: PipeRef, error?: unknown): void {
    this.send(error === undefined ? { type: 'pipe_done', pipeId: ref.id, ok: true } : { type: 'pipe_done', pipeId: ref.id, ok: false, error: String(error) })
  }
  private async upload(call: Call, ref: PipeRef): Promise<void> {
    try {
      if (!call.invocation.stdin) throw new Error('missing command input pipe')
      await this.pipes.put(ref.url, call.invocation.stdin)
      this.report(ref)
    } catch (error) { this.report(ref, error); throw error }
  }
  private async download(call: Call, ref: PipeRef): Promise<void> {
    try {
      for await (const bytes of await this.pipes.get(ref.url)) await call.writes.run(() => Promise.resolve(call.invocation.io.stdout(bytes)))
      this.report(ref)
    } catch (error) { this.report(ref, error); throw error }
  }
}
