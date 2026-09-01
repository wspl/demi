import { CommandRegistry, type BashEnvironmentOptions, type Host } from '@demicodes/shell'
import type { AgentProvider, ProviderSelection } from '@demicodes/provider'
import { AgentSession } from '../session/session'
import { createStandardAgentTools } from '../tools'
import { ChildSupervisor, injectSubagentCommand } from '../subagent/supervisor'
import { hostAgentSessionStore } from '../store/session-store'
import type { BlobStore } from '../store/media'
import type { ServerFrame } from '../protocol/frames'
import type { AgentHarness, AgentHarnessRuntime, AgentSessionStore } from '../types'
import { LiveSession } from './live-session'
import type { AgentServerSessionOptions, PrepareShell } from './server'

/** Everything the assembly pipeline needs from the server/binding configuration. */
export interface AssembleLiveSessionDeps {
  agent: AgentHarness<unknown>
  shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  sessionOptions: AgentServerSessionOptions
  prepareShell: PrepareShell | null
  notifyParentOnIdle: boolean
  sessionStore: ((agentSessionId: string, host: Host) => AgentSessionStore<unknown>) | null
  blobs: BlobStore | null
}

export interface AssembleLiveSessionParams {
  agentSessionId: string
  cwd: string
  /** The already-constructed provider runtime plus the selection it came from. */
  provider: AgentProvider
  selection: ProviderSelection
}

export interface AssembledLiveSession {
  live: LiveSession
  /** True when the session was rebuilt from persisted rows (caller replays children after the open handshake). */
  restoring: boolean
}

/**
 * The session-assembly pipeline: resolve the store, decide restore vs fresh,
 * and build the supervisor / command tree / tools / runtime / AgentSession /
 * LiveSession aggregate. The three deferred references (`live`, `liveSink`,
 * `sessionRef`) exist because tools and the supervisor need handles to
 * objects constructed after them; they are assigned before this function
 * returns, and the closures only run once the session is live.
 */
export async function assembleLiveSession(
  deps: AssembleLiveSessionDeps,
  params: AssembleLiveSessionParams,
): Promise<AssembledLiveSession> {
  const { agent } = deps
  const { agentSessionId, cwd, provider, selection } = params

  // The default persistence lives in Host.store, so a Host is needed before
  // the restored state exists. Harnesses must tolerate host() being called
  // with initial state for store access (listConversations does the same).
  const initialState = agent.initialState()
  const provisionalHost = await agent.host({ state: initialState, cwd })
  const store = deps.sessionStore
    ? deps.sessionStore(agentSessionId, provisionalHost)
    : hostAgentSessionStore(provisionalHost.store, `agent-sessions/${agentSessionId}`, {
        blobs: deps.blobs ?? undefined,
      })
  const checkpoint = await store.load()
  const restoring = checkpoint !== null && checkpoint.harnessName === agent.name

  // One live state object, shared by the harness closures (host, commands,
  // prompts) and the session itself. On restore it carries the saved state.
  const state = restoring ? structuredClone(checkpoint.state) : initialState
  const harnessContext = { state, cwd }
  const harnessCommands = (await agent.commands?.({ ...harnessContext, agentSessionId })) ?? []
  const profiles = (await agent.agents?.(harnessContext)) ?? null

  let live: LiveSession | null = null
  const liveSink = (serverFrame: ServerFrame): void => {
    live?.sink(serverFrame)
  }
  // Root sessions get the subagent surface (`demi agent`); child sessions are
  // supervisor-built with a send-parent-only tree, so spawn is root-only.
  const supervisor = new ChildSupervisor<unknown>({
    agent,
    cwd,
    profiles,
    parentCommands: harnessCommands,
    shellOptions: deps.shellOptions,
    prepareShell: deps.prepareShell,
    sessionOptions: deps.sessionOptions,
    notifyParentOnIdle: deps.notifyParentOnIdle,
    store: provisionalHost.store,
    blobs: deps.blobs ?? undefined,
    emit: liveSink,
  })
  const commands = injectSubagentCommand(harnessCommands, supervisor.rootCommandNode())
  const commandRegistry = new CommandRegistry()
  for (const command of commands) commandRegistry.register(command)
  let sessionRef: AgentSession<unknown> | null = null
  const tools = createStandardAgentTools({
    environment: (ctx, handle) => {
      if (!live) throw new Error('AgentServer: session is not ready for shell access')
      return live.resolveEnvironment(ctx, handle)
    },
    scheduleYield: (ctx, durationMs) => {
      if (!sessionRef) throw new Error('AgentServer: session is not ready for yield scheduling')
      return sessionRef.scheduleYieldWakeup(durationMs, ctx.metadata)
    },
  })
  // Commands are fixed for the session's lifetime, so the rendered help is too.
  const commandsPrompt = commandRegistry.renderHelp()
  const runtime: AgentHarnessRuntime<unknown> = {
    harnessName: agent.name,
    initialState: () => agent.initialState(),
    systemPrompt: (ctx) => agent.systemPrompt({ ...ctx, commandsPrompt }),
    preamble: (ctx) => agent.preamble?.(ctx) ?? null,
    resolveReferences: (ctx, content) => agent.resolveReferences?.(ctx, content) ?? content,
    lifecycle: (event) => agent.lifecycle?.(event),
    tools: () => tools,
  }
  const session = restoring
    ? AgentSession.fromCheckpoint(
        { provider, runtime, checkpoint: { ...checkpoint, state } },
        { agentSessionId, store, ...deps.sessionOptions },
      )
    : new AgentSession(
        { provider, model: selection.model, cwd, runtime, state },
        { agentSessionId, store, ...deps.sessionOptions },
      )
  sessionRef = session
  supervisor.attachParent(session)
  live = new LiveSession({
    agentSessionId,
    session,
    supervisor,
    agent,
    commandRegistry,
    cwd,
    providerId: selection.providerId,
    shellOptions: deps.shellOptions,
    prepareShell: deps.prepareShell,
  })
  // A resumed session restores its model from the checkpoint; align it with the
  // model the client opened with (which may differ from when it was saved).
  if (restoring) session.updateModel(null, selection.model)

  return { live, restoring }
}
