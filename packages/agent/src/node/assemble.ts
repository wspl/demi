import type { ModelSelection, QueuedMessage } from '@demicodes/core'
import type { AgentProvider } from '@demicodes/provider'
import { CommandRegistry, RESERVED_COMMAND_NAMES, type Command, type ShellEnvironmentOptions } from '@demicodes/shell'
import type { ServerFrame } from '../protocol/frames'
import type { AgentServerSessionOptions, ShellEnvironmentFactory } from '../server/server'
import { AgentSession } from '../session/session'
import type { AgentDirectory } from '../subagent/directory'
import { ChildSupervisor, injectSubagentCommand } from '../subagent/supervisor'
import { createStandardAgentTools, type ShellPreviewBudget } from '../tools'
import type { AgentHarness, AgentHarnessRuntime, AgentNodeRecord, AgentTreeStore, SubagentProfile } from '../types'
import { SessionNode, type NodePolicy } from './node'

/** What every node of a server shares: the harness and the assembly options. */
export interface NodeDeps<State> {
  agent: AgentHarness<State>
  shellOptions: ShellEnvironmentOptions
  shellEnvironment: ShellEnvironmentFactory
  sessionOptions: AgentServerSessionOptions
  /** Whether a child of the root wakes an idle root by itself; deeper levels always do. */
  notifyParentOnIdle: boolean
  maxLiveSubagents: number
  shellPreviewBudgetTokens: ShellPreviewBudget | null
}

/** What every node of one tree shares: its store, its directory, its root, its frame sink. */
export interface TreeContext<State> {
  store: AgentTreeStore<State>
  directory: AgentDirectory<State>
  /** The root's id: Host resolution asks for the root, the execution target being the conversation's. */
  hostSessionId: string
  /** The harness profiles, resolved once per tree; the unnamed inherit profile is always available besides them. */
  profiles: SubagentProfile<State>[] | null
  emit(frame: ServerFrame): void
}

/** What makes one node itself: identity, relationship, configuration, policy. */
export interface NodeParams<State> {
  record: AgentNodeRecord
  cwd: string
  /** The provider runtime already constructed: the root's from the resolver, a child's cloned from its parent. */
  provider: AgentProvider
  model: ModelSelection
  /** The prompt the node speaks with: the harness for the root; a profile's or the parent's for a child. */
  prompt: Pick<AgentHarness<State>, 'systemPrompt' | 'preamble'>
  /** Appended to the preamble: a child's identity; null for the root. */
  preambleSuffix: string | null
  /** The harness commands before the `demi agent` injection, over the node's state. */
  commands(state: State): Promise<Command[]> | Command[]
  /** Shell environment on top of the server's: a child's identity variables. */
  shellEnv: Record<string, string>
  policy: NodePolicy
  /** A fresh node's first message, queued in the create commit; null for the root and for a node that exists. */
  firstMessage: QueuedMessage | null
  /** Wakes the owner when this node's children change; null for the root. */
  onJobsChanged: (() => void) | null
}

export interface AssembledNode<State> {
  node: SessionNode<State>
  /** True when the node came back from the store rather than being created. */
  restored: boolean
}

/**
 * The one way a node comes to exist: the root on `open`, a child on spawn,
 * either on restore. Loads the node from the tree store or creates it
 * (record and first checkpoint in one commit), then builds what every node
 * has — the supervisor of its children, the command tree with `demi agent`
 * grafted on, the standard tools over its shell environments, the harness
 * runtime, the session. The supervisor asks this function for children.
 */
export async function assembleNode<State>(
  deps: NodeDeps<State>,
  tree: TreeContext<State>,
  params: NodeParams<State>,
): Promise<AssembledNode<State>> {
  const { agent, shellPreviewBudgetTokens } = deps
  const stored = await tree.store.node(params.record.id)
  const sessionStore = tree.store.sessionStore(params.record.id)
  let checkpoint = stored ? await sessionStore.load() : null
  if (checkpoint && checkpoint.harnessName !== agent.name) {
    // Another harness's node under this id: the id starts over.
    await tree.store.deleteNode(params.record.id)
    checkpoint = null
  }
  const record = checkpoint && stored ? stored : params.record

  // One live state object, shared by the harness closures and the session.
  const state = checkpoint ? structuredClone(checkpoint.state) : agent.initialState()
  const harnessCommands = await params.commands(state)

  let node: SessionNode<State> | null = null
  const supervisor = new ChildSupervisor<State>({
    deps,
    tree,
    ownerId: record.id,
    cwd: params.cwd,
    parentCommands: harnessCommands,
    prompt: params.prompt,
    canSpawn: record.canSpawnSubagents,
    // A subagent parent has no product-side channel, so deeper levels always self-notify.
    notifyParentOnIdle: record.parentId === null ? deps.notifyParentOnIdle : true,
    onJobsChanged: params.onJobsChanged,
    assemble: (childParams) => assembleNode(deps, tree, childParams),
  })
  const commandRegistry = new CommandRegistry(RESERVED_COMMAND_NAMES)
  for (const command of injectSubagentCommand(harnessCommands, supervisor.rootCommandNode())) commandRegistry.register(command)
  // Commands are fixed for the node's lifetime, so the rendered help is too.
  const commandsPrompt = commandRegistry.renderHelp()

  const tools = createStandardAgentTools<State>({
    environment: (ctx, handle) => {
      if (!node) throw new Error('AgentServer: the node is not ready for shell access')
      return node.resolveEnvironment(ctx, handle)
    },
    scheduleYield: (ctx, durationMs) => {
      if (!node) throw new Error('AgentServer: the node is not ready for yield scheduling')
      return node.session.scheduleYieldWakeup(durationMs, ctx.metadata)
    },
    ...(shellPreviewBudgetTokens === null ? {} : { previewBudgetTokens: shellPreviewBudgetTokens }),
  })
  const runtime: AgentHarnessRuntime<State> = {
    harnessName: agent.name,
    initialState: () => agent.initialState(),
    systemPrompt: (ctx) => params.prompt.systemPrompt({ ...ctx, commandsPrompt }),
    preamble: async (ctx) => {
      const inherited = (await params.prompt.preamble?.(ctx)) ?? null
      if (!params.preambleSuffix) return inherited
      return inherited ? `${inherited}\n\n${params.preambleSuffix}` : params.preambleSuffix
    },
    resolveReferences: (ctx, content) => agent.resolveReferences?.(ctx, content) ?? content,
    lifecycle: (event) => agent.lifecycle?.(event),
    tools: () => tools,
  }
  const sessionOptions = { agentSessionId: record.id, store: sessionStore, ...deps.sessionOptions }
  const session = checkpoint
    ? AgentSession.fromCheckpoint<State>({ provider: params.provider, runtime, checkpoint: { ...checkpoint, state } }, sessionOptions)
    : new AgentSession<State>({ provider: params.provider, model: params.model, cwd: params.cwd, runtime, state }, sessionOptions)
  if (!checkpoint) {
    await tree.store.createNode(record, {
      changedBlocks: [],
      blockCount: 0,
      state: structuredClone(state),
      phase: 'idle',
      queue: params.firstMessage ? [params.firstMessage] : [],
      cwd: params.cwd,
      model: structuredClone(params.model),
      harnessName: agent.name,
    })
  }
  supervisor.attachParent(session)
  node = new SessionNode<State>({
    record,
    session,
    supervisor,
    agent,
    commandRegistry,
    cwd: params.cwd,
    hostSessionId: tree.hostSessionId,
    shellOptions: { ...deps.shellOptions, initialEnv: { ...deps.shellOptions.initialEnv, ...params.shellEnv } },
    shellEnvironment: deps.shellEnvironment,
    policy: params.policy,
    continuation: checkpoint
      ? { interrupted: checkpoint.phase !== 'idle', queued: checkpoint.queue }
      : params.firstMessage
        ? { interrupted: false, queued: [params.firstMessage] }
        : null,
  })
  return { node, restored: checkpoint !== null }
}
