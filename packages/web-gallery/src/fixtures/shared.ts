import type { SharedItem } from '@demicodes/web-ui/share/shared-items'
import { demoModel } from './blocks'
import { ago } from './time'

/**
 * A conversation's share (`sharing.md`): the question, the run of work it
 * took as a line of the steps not shared, the answer, which names a
 * subagent, shows an attachment, and names a file and a commit of the Host,
 * which a shared page shows as text; then a follow-up answered after a
 * longer run.
 */
export const SHARED_TITLE = 'Why the deploy failed'

export function sharedItems(): SharedItem[] {
  return [
    {
      type: 'user', id: 'shared-ask', turnId: 'shared-ask-turn', createdAt: ago(300_000), model: demoModel,
      content: [{ type: 'text', text: 'Why did last night’s deploy fail?' }], preamble: null,
    },
    { type: 'steps_not_shared', id: 'steps-not-shared:1', count: 3 },
    {
      type: 'text', id: 'shared-answer', createdAt: ago(200_000), model: demoModel,
      text: [
        'The health check timed out: the new build listens on port **3272**, and the check still asks **3271**. [Read the deploy logs](agent:3) found it in the job’s log.',
        '',
        '<attachment id="a1" />',
        '<attachment id="a3" />',
        '',
        'The port moved in [config.ts](src/server/config.ts#L12), in [Move the server port](commit:3f2a9c1). Point the check at 3272 and deploy again.',
      ].join('\n'),
    },
    {
      type: 'user', id: 'shared-fix', turnId: 'shared-fix-turn', createdAt: ago(150_000), model: demoModel,
      content: [{ type: 'text', text: 'Fix the check and deploy again.' }], preamble: null,
    },
    { type: 'steps_not_shared', id: 'steps-not-shared:4', count: 12 },
    {
      type: 'text', id: 'shared-fixed', createdAt: ago(60_000), model: demoModel,
      text: 'The check now asks port 3272, and the deploy went through: the health check answered in 40 ms.',
    },
  ]
}

/** An answer's share: the message, and the texts of its reply with the steps between them. */
export function sharedAnswerItems(): SharedItem[] {
  return sharedItems().slice(0, 3)
}
