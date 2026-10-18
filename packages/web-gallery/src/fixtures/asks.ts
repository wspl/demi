import type { Block } from '@demicodes/protocol'
import { demoModel, shellView, withEnd } from './blocks'
import { commentedConversation } from './comments'

/** One step a side question's answer takes, as its full conversation shows it. */
export interface AskStep {
  description: string
  script: string
  output: string
}

/** A scripted answer to a side question: the small model's title, its steps and its short answer. */
export interface AskScript {
  title: string
  steps: AskStep[]
  answer: string
}

/** The scripts of the approved prototype (`proto/comments-and-ask`). */
const SCRIPTS: AskScript[] = [
  {
    title: 'Why uploads time out',
    steps: [
      { description: 'Find the upload timeout', script: 'rg -n "timeout" app/src/net', output: 'app/src/net/client.ts:12:  timeoutMs: 30_000,\napp/src/net/upload.ts:48:  const response = await client.put(url, body)\n' },
      { description: 'Read the client, the upload and the server route', script: 'sed -n 1,80p app/src/net/client.ts app/src/net/upload.ts server/routes/uploads.ts', output: 'export const client = createClient({\n  timeoutMs: 30_000,\n  …\n' },
    ],
    answer: 'All API calls share one client, created in `client.ts` with a 30-second timeout. The upload calls `client.put` with the whole file, so a file that needs more than 30 seconds on the wire is always cut off.',
  },
  {
    title: 'Total retry wait',
    steps: [
      { description: 'Read the retry helper and its tests', script: 'sed -n 1,60p app/src/net/retry.ts app/src/net/retry.test.ts', output: 'export async function withRetry<T>(run: () => Promise<T>) {\n  …\n' },
    ],
    answer: 'Five attempts with waits of 1, 2, 4 and 8 seconds add up to about 15 seconds of waiting, plus the jitter. After that the upload stays in the queue and resumes the next time the app has a connection.',
  },
  {
    title: 'How long chunks are kept',
    steps: [
      { description: 'Find where chunks are stored', script: 'rg -n "chunk" server/routes/uploads.ts', output: 'server/routes/uploads.ts:31:  await store.appendChunk(id, offset, body)\nserver/routes/uploads.ts:64:  expireAfter: hours(24),\n' },
    ],
    answer: 'The server keeps the received chunks in its object store and drops them 24 hours after the last one arrived, not after the first. An upload that keeps making progress never expires.',
  },
]

/** Words that pick each script, in the order of `SCRIPTS`. */
const TOPICS = [/timeout|30|client|why/i, /wait|retr|back ?off|attempt|total/i, /keep|chunk|server|stor|expire|24/i]

/** The script for a question: the one whose topic its words name, else the `index`th in turn. */
export function askScript(question: string, index: number): AskScript {
  const topic = TOPICS.findIndex((words) => words.test(question))
  return SCRIPTS[topic >= 0 ? topic : index % SCRIPTS.length]!
}

/** The history a specimen's side question starts from: the commented conversation through its reply. */
export function askHistory(): Block[] {
  return commentedConversation()
}

let made = 0
const at = () => new Date(Date.now() + made++).toISOString()

/** A side question's message, as its transcript keeps it: the first quotes the words before the question. */
export function askMessage(id: string, index: number, quote: string, question: string): Block {
  const quoted = quote.split('\n').map((line) => `> ${line}`).join('\n')
  return {
    type: 'user',
    id: `${id}-q${index}`,
    // The backend names a side question's first message by its id.
    turnId: index === 0 ? id : `${id}-turn${index}`,
    createdAt: at(),
    model: demoModel,
    content: [{ type: 'text', text: index === 0 ? `${quoted}\n\n${question}` : question }],
    preamble: null,
  }
}

/** One completed step of a turn. */
export function askStep(id: string, turn: number, index: number, step: AskStep): Block {
  return withEnd({
    type: 'tool_call',
    id: `${id}-s${turn}-${index}`,
    createdAt: at(),
    model: demoModel,
    toolUseId: `${id}-s${turn}-${index}-use`,
    toolName: 'shell',
    status: 'completed',
    input: JSON.stringify({ script: step.script, description: step.description }),
    output: [],
    view: shellView({ commandId: `${turn + 1}${index + 1}`, chunks: [{ stream: 'stdout', text: step.output }] }),
  })
}

export function askAnswer(id: string, turn: number, text: string): Block {
  return { type: 'text', id: `${id}-a${turn}`, createdAt: at(), model: demoModel, text }
}

/** The failure a specimen's turn ends with: the provider's overload, as a turn's record shows it. */
export function askFailure(id: string, turn: number): Block {
  return {
    type: 'error',
    id: `${id}-e${turn}`,
    createdAt: at(),
    model: demoModel,
    message: 'Anthropic API request failed with HTTP 529: Overloaded. The provider could not accept the request.',
    code: 'overloaded',
    diagnostics: {
      source: 'http',
      httpStatus: 529,
      providerCode: 'overloaded_error',
      clientRequestId: 'req_01J8Y3Q6ZKX9',
    },
  }
}
