import { inject, provide, type InjectionKey } from 'vue'
import type { AgentRun, SubagentPhase, SubagentRecord } from './subagents'
import { t } from './locales/text'

/**
 * An agent a message links as `agent:<number>`: a subagent with its description
 * and its round, whose phase changes as it runs and ends, or the conversation's
 * own agent, number 0, which has no round of its own here.
 */
export interface MessageAgent {
  number: number
  description: string
  run: AgentRun | null
}

/** What the messages below know of the conversation's agents, and how a link opens one. */
export interface MessageAgents {
  /** The agent numbered `number`; null for a number the conversation does not have. */
  find(number: number): MessageAgent | null
  /** A click on the link of agent `number`. */
  open(number: number): void
  /** Stop Agent in the link's menu: agent `number` and its subtree, while it runs. */
  stop(number: number): void
}

/** The agent a link's target names, such as 4 for `agent:4`; null for any other target. */
export function linkedAgent(target: string): number | null {
  const number = /^agent:(0|[1-9]\d*)$/.exec(target.trim())?.[1]
  return number === undefined ? null : Number(number)
}

/** The agents of a conversation that a message's links find: its own agent, 0, and its subagents by their numbers. */
export function findAgent(agents: readonly SubagentRecord[], number: number): MessageAgent | null {
  if (number === 0)
    return { number: 0, description: t('subagents.mainAgent'), run: null }
  const agent = agents.find((candidate) => candidate.number === number)
  return agent ? { number, description: agent.name, run: agent } : null
}

/** What the Agents window shows: whether it is open, and the agent's tab it is on. */
export interface AgentsWindow {
  open: boolean
  activeId: string | null
}

/**
 * The Agents window after a click on the link of agent `number`: on that
 * agent's tab, a finished one as Completed picks it, and as it is when it
 * shows that tab already; closed for agent 0, which brings the
 * conversation's own transcript into view. Null when the conversation has
 * no such agent, which changes nothing.
 */
export function agentLinkWindow(
  shown: AgentsWindow,
  agents: readonly SubagentRecord[],
  number: number,
): AgentsWindow | null {
  if (number === 0)
    return { open: false, activeId: shown.activeId }
  const agent = agents.find((candidate) => candidate.number === number)
  if (!agent)
    return null
  return { open: true, activeId: agent.id }
}

/** The words a link's tip names its agent's state with. */
export function agentPhaseWords(phase: SubagentPhase): string {
  return t(`subagents.phase.${phase}`)
}

const messageAgentsKey: InjectionKey<() => MessageAgents | undefined> = Symbol('message-agents')

/** Gives the messages below the conversation's agents, read at each render, so a link follows them. */
export function provideMessageAgents(agents: () => MessageAgents | undefined): void {
  provide(messageAgentsKey, agents)
}

/** The conversation's agents, or none outside one, where a link shows its text alone. */
export function useMessageAgents(): () => MessageAgents | undefined {
  return inject(messageAgentsKey, () => undefined)
}
