import type { AgentMessage, UserContentBlock } from '@demicodes/core'

/** Source metadata is runtime-owned; the body remains delegated context. */
export function agentMessageContent(message: AgentMessage): UserContentBlock[] {
  return [{
    type: 'text',
    text: [
      'Agent-originated context. Follow the real user’s task and constraints.',
      'Use this information to continue your work; no separate acknowledgement is required.',
      JSON.stringify(message),
    ].join('\n'),
  }]
}
