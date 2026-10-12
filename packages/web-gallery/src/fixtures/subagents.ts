import type { AgentMessage, Block, JobModel } from '@demicodes/protocol'
import type { AgentRun, SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import { demoModel, inRun, screenshotTool, shellView, thinkingText } from './blocks'
import { wholeHistory } from '@demicodes/web-ui/agent/history'
import { ago } from './time'

function user(id: string, createdAt: string, text: string): Block {
  return {
    type: 'user',
    id,
    turnId: `${id}-turn`,
    createdAt,
    model: demoModel,
    content: [
      {
        type: 'text',
        text,
      },
    ],
    preamble: null,
  }
}

function thinking(id: string, createdAt: string, text: string): Block {
  return {
    type: 'thinking',
    id,
    createdAt,
    model: demoModel,
    text,
    signature: null,
  }
}

function text(id: string, createdAt: string, body: string): Block {
  return {
    type: 'text',
    id,
    createdAt,
    model: demoModel,
    text: body,
  }
}

function shell(
  id: string,
  createdAt: string,
  description: string,
  stdout: string,
  status: 'executing' | 'completed',
  script = `echo ${description}`,
): Block {
  return {
    type: 'tool_call',
    id,
    createdAt,
    model: demoModel,
    toolUseId: `${id}-use`,
    toolName: 'shell',
    input: JSON.stringify({
      script,
      description,
    }),
    status,
    output: [],
    view: shellView({
      commandId: `cmd-${id}`,
      chunks: [
        {
          stream: 'stdout',
          text: stdout,
        },
      ],
    }),
  }
}

const OPUS: JobModel = { provider: 'Anthropic', model: 'Claude Opus 5.5', effort: 'high', fast: false }
const SONNET: JobModel = { provider: 'Anthropic', model: 'Claude Sonnet 5.5', effort: 'medium', fast: false }
const HAIKU: JobModel = { provider: 'Anthropic', model: 'Claude Haiku 5', effort: 'low', fast: false }

/**
 * What a gallery child runs on (`subagents.md` § What an agent runs on): its
 * profile, none for the inherit profile, its model, and its round and fork
 * when they are not the first and none.
 */
function runsOn(
  profile: string | null,
  model: JobModel,
  more: Partial<Pick<AgentRun, 'round' | 'forkedFrom'>> = {},
): Pick<AgentRun, 'profile' | 'model' | 'round' | 'forkedFrom'> {
  return { profile, model, round: 1, forkedFrom: null, ...more }
}

/** What Check the logout path tells its parent while it works. */
const LOGOUT_UPDATE = 'Logout clears the renamed cookie in `auth.ts`. I am checking the **redirect** next.'
/** Update helper comments' last answer, its result. */
const COMMENTS_RESULT = [
  'Comments on the cookie helper now say `session`. The helper is unchanged.',
  '',
  '## Validation',
  ...Array.from({ length: 12 }, (_, i) => `- Check ${i + 1}: source identity, content, and message order remain intact.`),
].join('\n')
/** Why Run the fixture server failed. */
const FIXTURE_FAILURE = 'The fixture could not reach the test server. No files were changed.\n\n`ECONNREFUSED 127.0.0.1:3271`'
/** What Find the old suite paths tells its parent while the parent fixes the mjsunit tests. */
const SUITE_PATHS_UPDATE = 'The status file still lists the suites under `test/mjsunit/wasm-old/`; they moved to `test/mjsunit/wasm/`.'

/**
 * The receipts the gallery's children sent their parent, one of each kind
 * (`subagents.md` § Product rendering): each names the block of its
 * child's transcript it came from, an update the call that sent it, a
 * completion the last answer; the stopped child gave none.
 */
export const agentReceiptMessages: AgentMessage[] = [
  {
    id: 'receipt-logout',
    sender: { id: 'ag-logout', number: 2, description: 'Check the logout path', round: 1 },
    recipientId: 'gallery-parent',
    timestamp: '2026-09-12T12:00:00.000Z',
    content: LOGOUT_UPDATE,
    event: { type: 'message' },
    origin: 'ag-logout-send',
  },
  {
    id: 'subagent:ag-comments:1',
    sender: { id: 'ag-comments', number: 4, description: 'Update helper comments', round: 2 },
    recipientId: 'gallery-parent',
    timestamp: '2026-09-12T12:01:00.000Z',
    content: COMMENTS_RESULT,
    event: { type: 'completion', outcome: 'completed' },
    origin: 'ag-comments-text',
  },
  {
    id: 'subagent:ag-fixture:1',
    sender: { id: 'ag-fixture', number: 6, description: 'Run the fixture server', round: 1 },
    recipientId: 'gallery-parent',
    timestamp: '2026-09-12T12:02:00.000Z',
    content: FIXTURE_FAILURE,
    event: { type: 'completion', outcome: 'failed' },
    origin: 'ag-fixture-error',
  },
  {
    id: 'subagent:ag-ci:1',
    sender: { id: 'ag-ci', number: 5, description: 'CI screenshot review', round: 1 },
    recipientId: 'gallery-parent',
    timestamp: '2026-09-12T12:03:00.000Z',
    content: 'The parent stopped this round before validation finished.',
    event: { type: 'completion', outcome: 'aborted' },
  },
]

/**
 * A message the main agent sent its child, in the child's transcript: the
 * receipt names the main agent, whose name opens the call that sent it in
 * the main transcript.
 */
export const mainAgentMessage: AgentMessage = {
  id: 'receipt-main',
  sender: { id: 'gallery-parent', number: 0, description: 'root session', round: 1 },
  recipientId: 'ag-logout',
  timestamp: '2026-09-12T12:04:00.000Z',
  content: 'Also check that logout clears the CSRF cookie.',
  event: { type: 'message' },
  origin: 'parent-send',
}

/** Links whose names take more than a line in a narrow column, in the prose and in a table's cell. */
export const agentLinksWrapMarkdown = `I asked [同名文件能重复粘贴；Agents chip 只在运行时显示；任务说明渲染 Markdown](agent:4) to check the three fixes together, and [Make the fixture server pick a free port before it starts listening](agent:6) to try again.

| Agent | Task |
| --- | --- |
| 4 | [同名文件能重复粘贴；Agents chip 只在运行时显示；任务说明渲染 Markdown](agent:4) |
| 6 | Retried by [Make the fixture server pick a free port before it starts listening](agent:6) |`

/** A reply that links the agents of `gallerySubagents` by their numbers, and one the conversation does not have. */
export const agentLinksMarkdown = `I split the login work four ways. [Find the cookie assertion](agent:1) is still looking, [Update helper comments](agent:4) finished, [Run the fixture server](agent:6) failed on its port, and I stopped [CI screenshot review](agent:5) once the cause was clear. Each reports to the [main agent](agent:0); [Snapshot fonts](agent:9) was never started.

已经告诉[负责的 Agent](agent:4)：注释改完了。`

/** The update Find the old suite paths sent while the `work` turn ran, naming the call that sent it. */
export const suitePathsUpdate: AgentMessage = {
  id: 'receipt-suite-paths',
  sender: { id: 'ag-suite', number: 3, description: 'Find the old suite paths', round: 1 },
  recipientId: 'gallery-work',
  timestamp: '2026-09-12T12:05:00.000Z',
  content: SUITE_PATHS_UPDATE,
  event: { type: 'message' },
  origin: 'ag-suite-send',
}

/** The child the `work` turn hears from, still at work: its search, and the call that sent its update. */
export function workSubagent(): SubagentRecord {
  return {
    id: 'ag-suite',
    number: 3,
    name: 'Find the old suite paths',
    ...runsOn('research', SONNET),
    phase: 'running',
    sessionPhase: 'running',
    startedAt: ago(60_000),
    failures: {},
    pendingCalls: [],
    waitingSince: null,
    history: wholeHistory([
      user('ag-suite-user', ago(60_000), 'Find the suite paths test/mjsunit/mjsunit.status still names that no longer exist.'),
      ...inRun(shell(
        'ag-suite-tool',
        ago(50_000),
        'Find the suite paths the status file names',
        'test/mjsunit/mjsunit.status:12:  wasm-old/*: [SKIP]\n',
        'completed',
        'rg -n "wasm-old" test/mjsunit/mjsunit.status',
      ),
      shell(
        'ag-suite-send',
        ago(40_000),
        'Tell the parent where the old suite paths are',
        'sent to 0\n',
        'completed',
        `demi agent send parent <<'EOF'\n${SUITE_PATHS_UPDATE}\nEOF`,
      )),
    ]),
  }
}

export function gallerySubagents(): SubagentRecord[] {
  return [
    {
      id: 'ag-cookie',
      number: 1,
      name: 'Find the cookie assertion',
      ...runsOn(null, OPUS),
      phase: 'running',
      // Its turn ended while the search it started runs on: it shows no
      // Requesting row (`runtime.md` § Frame protocol).
      sessionPhase: 'idle',
      startedAt: ago(134_000),
      failures: {},
      pendingCalls: [],
      waitingSince: null,
      history: wholeHistory([
        user(
          'ag-cookie-user',
          ago(134_000),
          'Find the expect that still looks for sid in auth.test.ts. Do not edit yet.',
        ),
        // A screenshot the child viewed, which opens in the viewer over the Agents window.
        ...inRun(thinking('ag-cookie-think', ago(120_000), thinkingText), {
          ...screenshotTool,
          id: 'ag-cookie-screenshot',
          toolUseId: 'ag-cookie-screenshot-use',
          createdAt: ago(100_000),
        }, shell(
          'ag-cookie-tool',
          ago(90_000),
          'Find the old cookie name in the login test',
          'packages/web/src/auth.test.ts:18:    expect(cookie.name).toBe("sid")\n',
          'completed',
        )),
        text(
          'ag-cookie-text',
          ago(80_000),
          'The wider search for the old cookie name runs on; I will read its matches when it ends.',
        ),
      ]),
    },
    {
      id: 'ag-logout',
      number: 2,
      name: 'Check the logout path',
      ...runsOn('frontend', { ...SONNET, effort: 'high', fast: true }, { forkedFrom: 1 }),
      phase: 'running',
      sessionPhase: 'running',
      startedAt: ago(48_000),
      failures: {},
      pendingCalls: [],
      // The call's result went back 31 seconds ago and the model has not
      // answered yet: its run's row shimmers meanwhile, and the server tells
      // the wait from then (`runtime.md` § Waiting for the model).
      waitingSince: Date.now() - 31_000,
      history: wholeHistory([
        user(
          'ag-logout-user',
          ago(48_000),
          'See whether logout still clears the renamed session cookie.',
        ),
        text(
          'ag-logout-text',
          ago(40_000),
          'Logout should drop the session cookie. I will look for where the helper clears it.',
        ),
        ...inRun(shell(
          'ag-logout-tool',
          ago(36_000),
          'Find where logout clears the session cookie',
          'packages/web/src/auth.ts:42:  clearCookie(SESSION_COOKIE)\n',
          'completed',
        ),
        shell(
          'ag-logout-send',
          ago(33_000),
          'Tell the parent where logout clears the cookie',
          'sent to 0\n',
          'completed',
          `demi agent send parent <<'EOF'\n${LOGOUT_UPDATE}\nEOF`,
        )),
      ]),
    },
    {
      id: 'ag-snapshot',
      number: 7,
      name: 'Snapshot strings',
      ...runsOn('quick', { ...HAIKU, effort: 'off' }),
      phase: 'running',
      sessionPhase: 'running',
      startedAt: ago(12_000),
      failures: {},
      pendingCalls: [],
      // Its turn began 12 seconds ago and the model has not answered yet:
      // the view says Requesting from then, however often it opens
      // (`runtime.md` § Waiting for the model).
      waitingSince: Date.now() - 12_000,
      history: wholeHistory([
        user(
          'ag-snapshot-user',
          ago(12_000),
          'List snapshot strings that still mention the old cookie name.',
        ),
      ]),
    },
    {
      id: 'ag-comments',
      number: 4,
      name: 'Update helper comments',
      ...runsOn(null, OPUS, { round: 2 }),
      phase: 'completed',
      sessionPhase: 'idle',
      startedAt: ago(420_000),
      endedAt: ago(240_000),
      failures: {},
      pendingCalls: [],
      waitingSince: null,
      history: wholeHistory([
        user(
          'ag-comments-user',
          ago(420_000),
          'Update comments on the cookie helper to say session, not sid. Leave the code alone.',
        ),
        ...inRun(thinking(
          'ag-comments-think',
          ago(400_000),
          'Comments only. The helper already writes the new header.',
        ), shell(
          'ag-comments-tool',
          ago(360_000),
          'Open the cookie helper',
          'packages/web/src/cookie.ts\n',
          'completed',
        )),
        text(
          'ag-comments-text',
          ago(240_000),
          COMMENTS_RESULT,
        ),
      ]),
    },
    {
      id: 'ag-fixture',
      number: 6,
      name: 'Run the fixture server',
      ...runsOn('browser', SONNET),
      phase: 'error',
      sessionPhase: 'idle',
      startedAt: ago(300_000),
      endedAt: ago(270_000),
      failures: {},
      pendingCalls: [],
      waitingSince: null,
      history: wholeHistory([
        user(
          'ag-fixture-user',
          ago(300_000),
          'Start the fixture server and run the login test against it.',
        ),
        text(
          'ag-fixture-text',
          ago(290_000),
          'I will start the fixture server on port 3271, then run the login test.',
        ),
        ...inRun(shell(
          'ag-fixture-tool',
          ago(285_000),
          'Start the fixture server',
          'Error: connect ECONNREFUSED 127.0.0.1:3271\n',
          'completed',
        )),
        {
          type: 'error',
          id: 'ag-fixture-error',
          createdAt: ago(270_000),
          model: demoModel,
          message: 'The fixture could not reach the test server: connect ECONNREFUSED 127.0.0.1:3271',
          code: null,
        },
      ]),
    },
    {
      id: 'ag-ci',
      number: 5,
      name: 'CI screenshot review',
      ...runsOn('review', { ...OPUS, effort: 'medium' }),
      phase: 'aborted',
      sessionPhase: 'idle',
      startedAt: ago(180_000),
      endedAt: ago(120_000),
      failures: {},
      pendingCalls: [],
      waitingSince: null,
      history: wholeHistory([
        user(
          'ag-ci-user',
          ago(180_000),
          'Describe what the CI screenshot shows about the login failure.',
        ),
        ...inRun(thinking(
          'ag-ci-think',
          ago(160_000),
          'The screenshot is an attachment on the parent turn.',
        )),
        {
          type: 'abort',
          id: 'ag-ci-abort',
          createdAt: ago(120_000),
          model: demoModel,
          isResumed: false,
        },
      ]),
    },
  ]
}

/** A brief as a spawn writes one: Markdown with headings, a list and a table, which the child's transcript renders as an answer. */
const RESEARCH_BRIEF = `# Research: why the login test flakes

Read the login suite and find what makes it fail about one run in ten. Do not edit any file.

## Look at

- \`packages/web/src/auth.test.ts\`, its cookie assertions
- The fixture server's start-up in \`test/fixtures/server.ts\`
- The last week of CI runs of \`login.spec\`

## Report

| What | How |
| --- | --- |
| The failing assertion | Its file and line |
| Its cause | One sentence |
| A fix | The smallest change that holds |

Send the report with \`demi agent send parent\` as soon as you have the cause.`

/** A child at work on a brief written in full Markdown. */
export function briefSubagent(): SubagentRecord {
  return {
    id: 'ag-flake',
    number: 8,
    name: 'Find why the login test flakes',
    ...runsOn('research', SONNET),
    phase: 'running',
    sessionPhase: 'running',
    startedAt: ago(30_000),
    failures: {},
    pendingCalls: [],
    waitingSince: null,
    history: wholeHistory([
      user('ag-flake-user', ago(30_000), RESEARCH_BRIEF),
      text('ag-flake-text', ago(20_000), 'I will start with the cookie assertions, then the fixture server.'),
    ]),
  }
}
