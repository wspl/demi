import type { MessageAgents } from '../agent/agent-links'
import { t as agentText } from '../agent/locales/text'

/** A subagent a share's snapshot holds, as `GET /shares/:token` lists it for its transcript's header. */
export interface SharedAgent {
  number: number
  description: string
  /** The profile's name; null for the inherit profile. */
  profile: string | null
}

/** The name a shared subagent's header shows: its description, or its number when it has none, as a receipt names it. */
export function sharedAgentName(agent: SharedAgent): string {
  return agent.description.trim() || agentText('transcript.receipt.unnamed', { number: agent.number })
}

/**
 * The agents a shared transcript's `agent:` links find (`sharing.md` § What
 * a share holds): the conversation's own, 0, and each subagent the snapshot
 * holds, none of them running; `open` shows the one clicked, 0 the
 * conversation's own transcript. A number the snapshot lacks finds none, so
 * its link stays text.
 */
export function sharedMessageAgents(agents: readonly SharedAgent[], open: (number: number) => void): MessageAgents {
  return {
    find(number) {
      if (number === 0) {
        return { number: 0, description: agentText('subagents.mainAgent'), run: null }
      }
      const agent = agents.find((candidate) => candidate.number === number)
      return agent ? { number, description: sharedAgentName(agent), run: null } : null
    },
    open(number) {
      if (number === 0 || agents.some((agent) => agent.number === number)) {
        open(number)
      }
    },
  }
}
