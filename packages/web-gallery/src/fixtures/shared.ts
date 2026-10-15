import type { Block } from '@demicodes/protocol'
import type { SharedAgent } from '@demicodes/web-ui/share/shared-agents'
import { demoModel, inRun, shellView } from './blocks'
import { ago } from './time'

/**
 * A conversation shared with its subagent (`sharing.md`): the question, a
 * thinking and a command, a subagent's report, and the answer, which links
 * the subagent, shows an attachment, and names a file and a commit of the
 * Host, which a shared page shows as text.
 */
export const SHARED_TITLE = 'Why the deploy failed'

export const sharedAgents: SharedAgent[] = [
  { number: 3, description: 'Read the deploy logs', profile: 'research' },
]

function shell(id: string, createdAt: string, description: string, script: string, stdout: string): Block {
  return {
    type: 'tool_call', id, createdAt, model: demoModel, toolUseId: `${id}-use`, toolName: 'shell',
    input: JSON.stringify({ script, description }), status: 'completed', output: [],
    view: shellView({ commandId: `cmd-${id}`, chunks: [{ stream: 'stdout', text: stdout }] }),
  }
}

export function sharedRootBlocks(): Block[] {
  return [
    {
      type: 'user', id: 'shared-ask', turnId: 'shared-ask-turn', createdAt: ago(300_000), model: demoModel,
      content: [{ type: 'text', text: 'Why did last night’s deploy fail?' }], preamble: null,
    },
    ...inRun(
      { type: 'thinking', id: 'shared-think', createdAt: ago(298_000), model: demoModel, text: 'The deploy job ran at 02:00. I should read its log and the health check it waits on.', signature: null },
      shell('shared-status', ago(290_000), 'Check the deploy job', 'systemctl status deploy', 'deploy.service: Failed with result \'exit-code\'.\n'),
      shell('shared-spawn', ago(280_000), 'Read the deploy logs in a subagent', 'demi agent --description "Read the deploy logs" --profile research', 'agent 3 started\n'),
    ),
    {
      type: 'text', id: 'shared-answer', createdAt: ago(200_000), model: demoModel,
      text: [
        'The health check timed out: the new build listens on port **3272**, and the check still asks **3271**. [Read the deploy logs](agent:3) found it in the job’s log.',
        '',
        '![The failed check](attachment:a1)',
        '',
        'The port moved in [config.ts](src/server/config.ts#L12), in [Move the server port](commit:3f2a9c1). Point the check at 3272 and deploy again.',
      ].join('\n'),
    },
  ]
}

export function sharedAgentBlocks(): Block[] {
  return [
    {
      type: 'user', id: 'shared-agent-ask', turnId: 'shared-agent-ask-turn', createdAt: ago(280_000), model: demoModel,
      content: [{ type: 'text', text: 'Read the deploy job’s log from last night and tell me why it failed.' }], preamble: null,
    },
    ...inRun(shell('shared-agent-log', ago(270_000), 'Read the deploy log', 'journalctl -u deploy --since 02:00', 'health check http://127.0.0.1:3271/health: connection refused\n')),
    { type: 'text', id: 'shared-agent-answer', createdAt: ago(260_000), model: demoModel, text: 'The health check asked port 3271 and was refused; the server now listens on 3272.' },
  ]
}
