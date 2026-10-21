import { NO_COST } from '@demicodes/web-ui/agent/context-cost'
import type { AgentMessage, Block, JobModel } from '@demicodes/protocol'
import type { AgentRun, SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import { demoModel, inRequests, inRun, screenshotTool, shellView, thinkingText, withEnd } from './blocks'
import { wholeHistory } from '@demicodes/web-ui/agent/history'
import { usageAt } from './catalog'
import { ago } from './time'
import { SLOT_NODE, slotBlocks } from './request-changes'
import { SLOT_ROOT, WORKSPACE_ROOT } from './workspace'

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
  return withEnd({
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
  })
}

const OPUS: JobModel = { providerId: 'anthropic', provider: 'Anthropic', model: 'Claude Opus 5.5', effort: 'high', fast: false }
const SONNET: JobModel = { providerId: 'anthropic', provider: 'Anthropic', model: 'Claude Sonnet 5.5', effort: 'medium', fast: false }
const HAIKU: JobModel = { providerId: 'anthropic', provider: 'Anthropic', model: 'Claude Haiku 5.5', effort: 'low', fast: false }

/**
 * What a gallery child runs on: its
 * profile, none for the inherit profile, its model, and its round, fork and
 * working directory when they are not the first, none and the conversation's.
 */
function runsOn(
  profile: string | null,
  model: JobModel,
  more: Partial<Pick<AgentRun, 'round' | 'forkedFrom' | 'cwd'>> = {},
): Pick<AgentRun, 'profile' | 'model' | 'round' | 'forkedFrom' | 'cwd'> {
  return { profile, model, round: 1, forkedFrom: null, cwd: WORKSPACE_ROOT, ...more }
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
 * The receipts the gallery's children sent their parent, one of each kind:
 * each names the block of its
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
export const agentLinksWrapMarkdown = `I asked [同名商品能重复加入购物车；优惠券只在结账时显示；订单备注渲染 Markdown](agent:4) to check the three fixes together, and [Make the fixture server pick a free port before it starts listening](agent:6) to try again.

| Agent | Task |
| --- | --- |
| 4 | [同名商品能重复加入购物车；优惠券只在结账时显示；订单备注渲染 Markdown](agent:4) |
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
    onPath: true,
    parent: 'gallery-work',
    name: 'Find the old suite paths',
    ...runsOn('research', SONNET),
    phase: 'running',
    sessionPhase: 'running',
    contextUsage: usageAt(0.34),
    startedAt: ago(60_000),
    failures: {},
    pendingCalls: [],
    waitingSince: null,
    cost: NO_COST,
    shownAt: null,
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
      onPath: true,
      parent: 'gallery-work',
      name: 'Find the cookie assertion',
      ...runsOn(null, OPUS),
      phase: 'running',
      // Its turn ended while the search it started runs on: it shows no
      // Requesting row.
      sessionPhase: 'idle',
      contextUsage: usageAt(0.58),
      startedAt: ago(134_000),
      failures: {},
      pendingCalls: [],
      waitingSince: null,
      cost: NO_COST,
      shownAt: null,
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
      onPath: true,
      parent: 'gallery-work',
      name: 'Check the logout path',
      ...runsOn('frontend', { ...SONNET, effort: 'high', fast: true }, { forkedFrom: 1, cwd: SLOT_ROOT }),
      phase: 'running',
      sessionPhase: 'running',
      contextUsage: usageAt(0.22),
      startedAt: ago(48_000),
      failures: {},
      pendingCalls: [],
      // The call's result went back 31 seconds ago and the model has not
      // answered yet: its run's row shimmers meanwhile, and the server tells
      // the wait from then.
      waitingSince: Date.now() - 31_000,
      cost: NO_COST,
      shownAt: null,
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
      onPath: true,
      parent: 'gallery-work',
      name: 'Snapshot strings',
      ...runsOn('quick', { ...HAIKU, effort: 'off' }),
      phase: 'running',
      sessionPhase: 'running',
      contextUsage: usageAt(0.41),
      startedAt: ago(12_000),
      failures: {},
      pendingCalls: [],
      // Its turn began 12 seconds ago and the model has not answered yet:
      // the view says Requesting from then, however often it opens.
      waitingSince: Date.now() - 12_000,
      cost: NO_COST,
      shownAt: null,
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
      onPath: true,
      parent: 'gallery-work',
      name: 'Update helper comments',
      ...runsOn(null, OPUS, { round: 2 }),
      phase: 'completed',
      sessionPhase: 'idle',
      contextUsage: usageAt(0.47),
      startedAt: ago(420_000),
      endedAt: ago(240_000),
      failures: {},
      pendingCalls: [],
      waitingSince: null,
      cost: NO_COST,
      shownAt: null,
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
      onPath: true,
      parent: 'gallery-work',
      name: 'Run the fixture server',
      ...runsOn('browser', SONNET),
      phase: 'error',
      sessionPhase: 'idle',
      contextUsage: usageAt(0.12),
      startedAt: ago(300_000),
      endedAt: ago(270_000),
      failures: {},
      pendingCalls: [],
      waitingSince: null,
      cost: NO_COST,
      shownAt: null,
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
      onPath: true,
      parent: 'gallery-work',
      name: 'CI screenshot review',
      ...runsOn('review', { ...OPUS, effort: 'medium' }),
      phase: 'aborted',
      sessionPhase: 'idle',
      contextUsage: usageAt(0.09),
      startedAt: ago(180_000),
      endedAt: ago(120_000),
      failures: {},
      pendingCalls: [],
      waitingSince: null,
      cost: NO_COST,
      shownAt: null,
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
    onPath: true,
    parent: 'gallery-work',
    name: 'Find why the login test flakes',
    ...runsOn('research', SONNET),
    phase: 'running',
    sessionPhase: 'running',
    contextUsage: usageAt(0.27),
    startedAt: ago(30_000),
    failures: {},
    pendingCalls: [],
    waitingSince: null,
    cost: NO_COST,
    shownAt: null,
    history: wholeHistory([
      user('ag-flake-user', ago(30_000), RESEARCH_BRIEF),
      text('ag-flake-text', ago(20_000), 'I will start with the cookie assertions, then the fixture server.'),
    ]),
  }
}

/**
 * A child spawned into another checkout
 * (`crates/agent-server/src/node/mod.rs`): its links and its file change name
 * that checkout's files, and the folder in its status bar is where it works
 * now.
 */
export function slotSubagent(): SubagentRecord {
  return {
    id: SLOT_NODE,
    number: 9,
    onPath: true,
    parent: 'gallery-work',
    name: 'Rename the cookie in the slot',
    ...runsOn('frontend', SONNET, { cwd: `${SLOT_ROOT}/src/auth` }),
    phase: 'running',
    sessionPhase: 'running',
    contextUsage: usageAt(0.18),
    startedAt: ago(300_000),
    failures: {},
    pendingCalls: [],
    waitingSince: null,
    cost: NO_COST,
    shownAt: null,
    history: wholeHistory(slotBlocks()),
  }
}

/** The conversation's own agent, which the bubble specimens' children report to. */
export const BUBBLES_ROOT = 'gallery-bubbles'

/**
 * A message the agent `sender` sent the agent `recipient`, from its call
 * `origin`: a step of no run, as the backend places one from above or beside
 * the recipient (`inRun` makes it one from below).
 */
function agentSays(
  id: string,
  sender: { id: string, number: number, description: string },
  recipient: string,
  createdAt: string,
  content: string,
  origin: string,
): Block {
  const message: AgentMessage = {
    id,
    sender: { ...sender, round: 1 },
    recipientId: recipient,
    timestamp: createdAt,
    content,
    event: { type: 'message' },
    origin,
  }
  return { type: 'agent_message', id, turnId: `${id}-turn`, createdAt, model: demoModel, message }
}

const NOTES = { id: 'ag-notes', number: 3, description: 'Tidy the release notes' }
const REVIEW = { id: 'ag-review', number: 4, description: 'Review the notes' }
const LINKS = { id: 'ag-links', number: 5, description: 'Check the links' }
const MAIN = { id: BUBBLES_ROOT, number: 0, description: 'root session' }

/**
 * Tidy the release notes' transcript: its brief, its work, a message from the
 * main agent and one from its sibling Review the notes, and with `report`, the
 * report of its own child Check the links. The tree: Tidy the release notes
 * and Review the notes are the main agent's children, Check the links is Tidy
 * the release notes' child. A message from above or beside it ends a run, as
 * the user's words do.
 */
function notesBlocks(report: boolean): Block[] {
  return [
    user('ag-notes-user', ago(240_000), 'Tidy the release notes in `docs/RELEASE.md`: one heading per version, newest first, and fix any broken table.'),
    ...inRun(
      shell('ag-notes-read', ago(230_000), 'Read the release notes', '# Release notes\n## 2.4.0\n…', 'completed', 'cat docs/RELEASE.md'),
      shell('ag-notes-sort', ago(220_000), 'List the version headings', '## 2.4.0\n## 2.2.0\n## 2.3.0', 'completed', "grep -n '^## ' docs/RELEASE.md"),
      shell('ag-notes-spawn', ago(210_000), 'Start a check of the links', 'Spawned agent 5', 'completed', 'demi agent spawn --description "Check the links"'),
    ),
    agentSays('ag-notes-main', MAIN, NOTES.id, ago(200_000), 'Also keep the old **Upgrading** heading; other pages link to it.', 'gallery-bubbles-send'),
    text('ag-notes-ack', ago(190_000), 'I will keep the Upgrading heading and move the versions under it.'),
    agentSays('ag-notes-review', REVIEW, NOTES.id, ago(150_000), 'The table on line 40 is broken: its second row has one cell too few.', 'ag-review-send'),
    ...(report
      ? [{
          type: 'agent_message',
          id: 'subagent:ag-links:1',
          run: 'subagent:ag-links:1',
          turnId: 'ag-notes-links-turn',
          createdAt: ago(120_000),
          model: demoModel,
          message: {
            id: 'subagent:ag-links:1',
            sender: { ...LINKS, round: 1 },
            recipientId: NOTES.id,
            timestamp: ago(120_000),
            content: 'All 14 links resolve.',
            event: { type: 'completion', outcome: 'completed' },
            origin: 'ag-links-text',
          },
        } satisfies Block]
      : []),
    text('ag-notes-text', ago(100_000), 'The versions are in order and the table on line 40 has its missing cell.'),
  ]
}

/** A bubble specimen's agent: one of the tree's, finished, with its transcript. */
function bubbleAgent(agent: { id: string, number: number, description: string }, blocks: Block[]): SubagentRecord {
  return {
    id: agent.id,
    number: agent.number,
    onPath: true,
    parent: 'gallery-work',
    name: agent.description,
    ...runsOn('docs', SONNET),
    phase: 'completed',
    sessionPhase: 'idle',
    contextUsage: null,
    startedAt: ago(240_000),
    endedAt: ago(90_000),
    failures: {},
    pendingCalls: [],
    waitingSince: null,
    cost: NO_COST,
    shownAt: null,
    history: wholeHistory(blocks),
  }
}

/**
 * The agents of the bubble specimens, Tidy the release notes first; with
 * `report`, its child's report is in its transcript.
 */
export function bubbleAgents(report: boolean): SubagentRecord[] {
  return [
    bubbleAgent(NOTES, notesBlocks(report)),
    bubbleAgent(REVIEW, [
      user('ag-review-user', ago(220_000), 'Review `docs/RELEASE.md` and tell whoever tidies it what is wrong.'),
      ...inRun(shell('ag-review-send', ago(150_000), 'Tell Tidy the release notes of the broken table', 'Sent', 'completed', 'demi agent send 3')),
      text('ag-review-text', ago(140_000), 'I told agent 3 about the broken table.'),
    ]),
    bubbleAgent(LINKS, [
      user('ag-links-user', ago(210_000), 'Check that every link in `docs/RELEASE.md` resolves.'),
      ...inRun(shell('ag-links-check', ago(180_000), 'Check the links', '14 links, 0 broken', 'completed', 'lychee docs/RELEASE.md')),
      text('ag-links-text', ago(120_000), 'All 14 links resolve.'),
    ]),
  ]
}

/**
 * The main agent's transcript beside them: a message from its child Tidy the
 * release notes, which arrives after its answer and so is a run of its own,
 * keeps its receipt row, as every message from below does.
 */
export function bubblesMainBlocks(): Block[] {
  return inRequests([
    user('bubbles-user', ago(260_000), 'Tidy the release notes and have someone review them.'),
    ...inRun(
      shell('bubbles-spawn', ago(250_000), 'Start the tidy and the review', 'Spawned agents 3 and 4', 'completed', 'demi agent spawn'),
      shell('gallery-bubbles-send', ago(200_000), 'Ask Tidy the release notes to keep the old heading', 'Sent', 'completed', 'demi agent send 3'),
    ),
    text('bubbles-started', ago(195_000), 'Tidy the release notes and Review the notes are at work. I asked the tidy to keep the Upgrading heading.'),
    ...inRun(agentSays('bubbles-from-notes', NOTES, BUBBLES_ROOT, ago(180_000), 'I will keep the Upgrading heading.', 'ag-notes-ack')),
    text('bubbles-text', ago(90_000), 'The release notes are tidied, and the review found one broken table, now fixed.'),
  ])
}
