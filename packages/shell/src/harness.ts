import type {
  AgentDisposeContext,
  AgentLifecycleEvent,
  AgentPromptContext,
  AgentReferenceResolveContext,
} from '@demi/base-agent'
import type { UserContentBlock } from '@demi/core'
import type { CommandSpec } from './command'
import type { Host } from './host'

export interface AgentHarnessContext<State> {
  state: State
  cwd: string
}

export interface AgentHarness<State = unknown> {
  name: string
  initialState(): State
  host(ctx: AgentHarnessContext<State>): Host
  commands?(ctx: AgentHarnessContext<State>): CommandSpec[]
  systemPrompt(ctx: AgentPromptContext<State>): string
  preamble?(ctx: AgentPromptContext<State>): string | null
  resolveReferences?(
    ctx: AgentReferenceResolveContext<State>,
    content: UserContentBlock[],
  ): Promise<UserContentBlock[]> | UserContentBlock[]
  lifecycle?(event: AgentLifecycleEvent<State>): Promise<void> | void
  dispose?(ctx: AgentDisposeContext<State>): Promise<void> | void
}
