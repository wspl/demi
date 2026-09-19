import type { z } from 'zod'
import type { AgentMessage, Block, ModelSelection, TokenUsage, UserContentBlock } from '@demicodes/core'
import { encodeRemoteReference } from '@demicodes/web-ui/agent/message-input/attachments'
import type { PendingSteerRenderBlock } from '@demicodes/web-ui/agent/pending-steers'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import type { shellToolViewSchema } from '@demicodes/web-ui/transport/protocol'

export type ShellView = z.infer<typeof shellToolViewSchema>

/**
 * A complete shell view, the shape `shell_exec` writes and every reader
 * validates. A specimen names only the parts it is about; the rest is what a
 * finished command leaves behind.
 */
export function shellView(
  parts: Partial<ShellView> & Pick<ShellView, 'chunks'>
): ShellView {
  const view: ShellView = {
    kind: 'shell',
    status: 'exited',
    shellId: 'shell-demo',
    commandId: 'cmd-demo',
    runningMs: 1_200,
    idleMs: 0,
    viewTruncated: false,
    ...parts,
  }
  // A command reports its exit code only once it has exited.
  if (view.status === 'exited')
    view.exitCode = parts.exitCode ?? 0
  return view
}

export const demoModel: ModelSelection = {
  providerId: 'anthropic',
  model: {
    id: 'demo-model',
    name: 'Demo model',
    contextWindow: 200_000,
    outputLimit: null,
    inputLimit: 180_000,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

export const agentReceiptMessages: AgentMessage[] = [
  { type: 'message' as const },
  { type: 'completion' as const, outcome: 'completed' as const },
  { type: 'completion' as const, outcome: 'failed' as const },
  { type: 'completion' as const, outcome: 'aborted' as const },
].map((event, index) => ({
  id: event.type === 'completion' ? `subagent:agent-ui-${index}:${1789224000000 + index}` : `receipt-${index}`,
  sender: { id: `agent-ui-${index}`, description: 'UI implementation', round: 1789224000000 + index },
  recipientId: 'gallery-parent',
  timestamp: '2026-09-12T12:00:00.000Z',
  content: index === 0
    ? 'The shared receipt component is ready. I am checking **keyboard expansion** and reconnect behavior.'
    : index === 1
      ? ['Implemented the shared component and verified the product and gallery.', '', '## Validation', ...Array.from({ length: 12 }, (_, i) => `- Check ${i + 1}: source identity, content, and message order remain intact.`)].join('\n')
      : index === 2
        ? 'The fixture could not reach the test server. No files were changed.\n\n`ECONNREFUSED 127.0.0.1:3271`'
        : 'The parent stopped this round before validation finished.',
  event,
}))

export const demoUsage: TokenUsage = {
  inputTokens: 42_000,
  outputTokens: 6_400,
  cacheReadTokens: 12_000,
  cacheWriteTokens: 800,
}

export const demoImageUrl = '/fixtures/attachment-thumb.png'

export const longUserText = [
  'The login test in packages/web/src/auth.test.ts started failing after we renamed the session cookie from sid to session. CI is red on main and on this branch.',
  'The helper still writes Set-Cookie correctly. The assertion is what drifted: it looks for a sid= prefix and a Session header that we no longer send.',
  'I pasted the failing log and a screenshot from the last GitHub run. The request is a POST to /login with an email and password; the response is 204 and a session cookie.',
  'Please keep the fix inside auth.test.ts. Do not rename the helper, do not touch cookie.ts, and do not add a second test file just to isolate the assertion.',
  'The expired-cookie case can wait for a follow-up. I only want the rename covered so we can merge the cookie change today.',
  'If you need more context, the old name leaked into two comments and one snapshot string. Comments can stay; the snapshot has to match the new header.',
  'I already tried updating the snapshot locally. bun test packages/web/src/auth.test.ts still fails on the name field, so the expect() is the one that is wrong.',
  'When you are done, leave the rest of the suite alone. A green login test is enough for this turn.',
  'If the file is longer than you expect, scroll — the important expect is near the bottom, after the helper setup and the fixture header.',
].join('\n\n')

function iso(offsetMs: number): string {
  return new Date(Date.now() - offsetMs).toISOString()
}

function toolCall(
  partial: Pick<ToolCallBlock, 'id' | 'toolName' | 'input' | 'status'> & Partial<ToolCallBlock>
): ToolCallBlock {
  return {
    type: 'tool_call',
    createdAt: iso(120_000),
    model: demoModel,
    toolUseId: `${partial.id}-use`,
    streamingOutput: [],
    output: [],
    view: null,
    ...partial,
  }
}

export const userPrompt: UserContentBlock[] = [
  {
    type: 'text',
    text: 'The login test in `packages/web/src/auth.test.ts` is failing after the session cookie rename. Keep the fix in that file.'
  },
]

export const steerPrompt: UserContentBlock[] = [
  {
    type: 'text',
    text: 'Do not touch the cookie helper. Only fix the assertion.'
  },
]

export const assistantMarkdown = `The cookie helper is fine. The test still expects \`sid\`.

I updated the assertion in [auth.test.ts](tests/login/auth.test.ts) and left [cookie.ts](src/auth/cookie.ts) alone. The login page the test drives, and the whole page as the test captured it:

![The login page](assets/photo.png)

![The whole login page](assets/page-full.png)

\`\`\`ts
expect(readSessionCookie(header)).toEqual({
  name: 'session',
  value: 'abc',
})
\`\`\`
`

export const thinkingText = `The cookie name changed from sid to session. The helper already writes the new header. The test is the one still looking for sid.`

export const shellTool = toolCall({
  id: 'tool-shell',
  toolName: 'shell_exec',
  status: 'completed',
  input: JSON.stringify({
    script: 'rg -n "sid" packages/web/src/auth.test.ts',
    description: 'Find the old cookie name in the login test',
  }),
  view: shellView({
    commandId: 'cmd-find',
    chunks: [
      {
        stream: 'stdout',
        text: 'packages/web/src/auth.test.ts:18:    expect(cookie.name).toBe("sid")\n'
      },
    ],
  }),
})

export const runningShellTool = toolCall({
  id: 'tool-shell-run',
  toolName: 'shell_exec',
  status: 'executing',
  input: JSON.stringify({
    script: 'bun test packages/web/src/auth.test.ts',
    description: 'Run the login test',
  }),
  view: shellView({
    commandId: 'cmd-run',
    status: 'running',
    chunks: [{ stream: 'stdout', text: 'bun test v1.2\n' }],
  }),
})

export const editingShellTool = toolCall({
  id: 'tool-shell-edit',
  toolName: 'shell_exec',
  status: 'completed',
  input: JSON.stringify({
    script: 'sed -i "s/sid/session/" packages/web/src/auth.test.ts && bun test packages/web/src/auth.test.ts',
    description: 'Rename the cookie in the login test',
  }),
  view: shellView({
    commandId: 'cmd-edit',
    chunks: [{ stream: 'stdout', text: 'bun test v1.2\n 3 pass\n 0 fail\n' }],
    files: [
      { path: 'packages/web/src/auth.test.ts', kind: 'modified', added: 12, removed: 3, edits: [{ kept: true }] },
      { path: 'packages/web/src/cookie.ts', kind: 'modified', added: 1, removed: 1, edits: [{ kept: true }] },
    ],
  }),
})

function fileChangeCase(
  id: string,
  description: string,
  files: ShellView['files'],
  status: ToolCallBlock['status'] = 'completed',
): ToolCallBlock {
  return toolCall({
    id: `tool-files-${id}`,
    toolName: 'shell_exec',
    status,
    input: JSON.stringify({ script: `demi-edit ${id}`, description }),
    view: shellView({
      commandId: `cmd-files-${id}`,
      chunks: [{ stream: 'stdout', text: 'done\n' }],
      files,
    }),
  })
}

/** One shell call per way a command can touch files, from the common single edit to the overflow. */
export const fileChangeCases: { variant: string, block: ToolCallBlock }[] = [
  {
    variant: 'one edit',
    block: fileChangeCase('one', 'Fix the cookie assertion', [
      { path: 'packages/web/src/auth.test.ts', kind: 'modified', added: 1, removed: 1, edits: [{ kept: true }] },
    ]),
  },
  {
    variant: 'new file',
    block: fileChangeCase('new', 'Add the session helper', [
      { path: 'packages/web/src/session.ts', kind: 'added', added: 40, removed: 0, edits: [{ kept: true }] },
    ]),
  },
  {
    variant: 'append only, not new',
    block: fileChangeCase('append', 'Note the rename in the readme', [
      { path: 'packages/web/README.md', kind: 'modified', added: 3, removed: 0, edits: [{ kept: true }] },
    ]),
  },
  {
    variant: 'contents unavailable',
    block: fileChangeCase('unavailable', 'Update the binary asset', [
      { path: 'assets/logo.png', kind: 'modified', added: 0, removed: 0, edits: [{ kept: false }] },
    ]),
  },
  {
    variant: 'two edits to one file',
    block: fileChangeCase('segments', 'Update the cookie across interleaved writes', [
      { path: 'packages/web/src/auth.test.ts', kind: 'modified', added: 2, removed: 2, edits: [{ kept: true }, { kept: true }] },
    ]),
  },
  {
    variant: 'a few, mixed',
    block: fileChangeCase('mixed', 'Move the cookie name into one module', [
      { path: 'packages/web/src/auth.test.ts', kind: 'modified', added: 12, removed: 3, edits: [{ kept: true }] },
      { path: 'packages/web/src/session.ts', kind: 'added', added: 40, removed: 0, edits: [{ kept: true }] },
      { path: 'packages/web/package.json', kind: 'modified', added: 1, removed: 1, edits: [{ kept: true }] },
    ]),
  },
  {
    variant: 'long names',
    block: fileChangeCase('long', 'Regenerate the snapshots', [
      { path: 'packages/web/src/__snapshots__/auth.test.ts.snap', kind: 'modified', added: 2, removed: 2, edits: [{ kept: true }] },
      { path: 'packages/web/src/components/ConversationListDropdownItemWithAVeryLongName.vue', kind: 'modified', added: 5, removed: 5, edits: [{ kept: true }] },
      { path: 'packages/web/src/components/ConversationListDropdownItemWithAVeryLongName.test.ts', kind: 'added', added: 120, removed: 0, edits: [{ kept: true }] },
    ]),
  },
  {
    variant: 'more than three rows',
    block: fileChangeCase('many', 'Rename the prop across every widget', Array.from({ length: 40 }, (_, i) => ({
      path: `packages/web/src/components/Widget${i + 1}.vue`,
      kind: i % 9 === 0 ? 'added' : 'modified',
      added: i + 1,
      removed: i % 3,
      edits: [{ kept: true }],
    }))),
  },
  {
    variant: 'still running',
    block: fileChangeCase('running', 'Apply the codemod', [], 'executing'),
  },
  {
    variant: 'no files',
    block: fileChangeCase('none', 'List the test files', []),
  },
]

const caseBlock = (variant: string): Block =>
  fileChangeCases.find((item) => item.variant === variant)!.block as Block

/**
 * A heavy refactor turn: many shell calls in a row, most touching one or two
 * files, one sweeping forty, one still running. The Changes view shows it
 * whole so the pills are judged in a real transcript, not one row at a time.
 */
export function changesDemoBlocks(): Block[] {
  const text = (id: string, offsetMs: number, body: string): Block => ({
    type: 'text',
    id,
    createdAt: iso(offsetMs),
    model: demoModel,
    text: body,
  })
  return [
    {
      type: 'user',
      id: 'changes-user',
      turnId: 'changes-turn',
      createdAt: iso(200_000),
      model: demoModel,
      content: [{
        type: 'text',
        text: 'Rename the `sid` cookie to `session` across packages/web. Keep every widget compiling and regenerate the snapshots.',
      }],
      preamble: null,
    },
    {
      type: 'thinking',
      id: 'changes-thinking',
      createdAt: iso(190_000),
      model: demoModel,
      text: 'The name lives in cookie.ts, the login test, the snapshots and a prop on every widget. Start from the helper, then sweep the widgets with a codemod.',
      signature: null,
    },
    shellTool as Block,
    caseBlock('one edit'),
    caseBlock('new file'),
    caseBlock('contents unavailable'),
    caseBlock('two edits to one file'),
    text('changes-text-1', 150_000, 'The helper now writes `session`. Sweeping the widgets next; each one reads the cookie name from a prop.'),
    caseBlock('more than three rows'),
    caseBlock('a few, mixed'),
    caseBlock('no files'),
    caseBlock('long names'),
    caseBlock('append only, not new'),
    text('changes-text-2', 60_000, 'Every widget compiles and the snapshots match. Applying the same codemod to the docs examples.'),
    caseBlock('still running'),
  ]
}

export const yieldTool = toolCall({
  id: 'tool-yield',
  toolName: 'yield',
  status: 'completed',
  input: JSON.stringify({ description: 'Wait for the next user turn' }),
})

export const statusTool = toolCall({
  id: 'tool-status',
  toolName: 'shell_status',
  status: 'completed',
  input: JSON.stringify(
    {
      commandId: 'cmd_1',
      description: 'Check long-running command'
    }
  ),
})

export const writeTool = toolCall({
  id: 'tool-write',
  toolName: 'shell_write',
  status: 'completed',
  input: JSON.stringify({ commandId: 'cmd_1', data: 'continue' }),
})

export const abortTool = toolCall({
  id: 'tool-abort',
  toolName: 'shell_abort',
  status: 'completed',
  input: JSON.stringify({ commandId: 'cmd_1' }),
})

export const errorTool = toolCall({
  id: 'tool-error',
  toolName: 'shell_exec',
  status: 'error',
  input: JSON.stringify({ script: 'false', description: 'Broken command' }),
  output: [{ type: 'text', text: 'exit 1\npermission denied: /tmp/locked\n' }],
})

/** One request and one answer: enough history for a failure to keep on screen. */
export function shortTranscriptBlocks(): Block[] {
  return [
    {
      type: 'user',
      id: 'short-user',
      turnId: 'short-turn',
      createdAt: iso(120_000),
      model: demoModel,
      content: [
        {
          type: 'text',
          text: 'Build a small minesweeper game with a timer and difficulty settings.',
        },
      ],
      preamble: null,
    },
    {
      type: 'text',
      id: 'short-answer',
      createdAt: iso(60_000),
      model: demoModel,
      text: 'The game is ready.\n\nChoose a difficulty, reveal cells and mark suspected mines. The first click is always safe, and the timer starts with the first move.',
    },
  ]
}

/** The provider error that ends a turn, as the transcript records it. */
export function generationErrorBlock(): Block {
  return {
    type: 'error',
    id: 'generation-error',
    createdAt: iso(30_000),
    model: demoModel,
    message: 'Anthropic API request failed with HTTP 529: Overloaded. The provider could not accept the request.',
    code: 'overloaded',
    diagnostics: {
      source: 'http',
      httpStatus: 529,
      providerCode: 'overloaded_error',
      clientRequestId: 'req_01J8Y3Q6ZKX5',
    },
  }
}

/** A steer typed while the turn runs. Renders after every transcript block, never among them. */
export const pendingSteerDemo: PendingSteerRenderBlock = {
  type: 'pending_steer',
  id: 'pending-steer-1',
  pendingSteerId: 'pending-1',
  content: steerPrompt,
}

export function transcriptDemoBlocks(): Block[] {
  const thinkingStartedAt = iso(18_000)
  const thinkingEndedAt = iso(10_000)

  return [
    {
      type: 'user',
      id: 'user-1',
      turnId: 'turn-1',
      createdAt: iso(120_000),
      model: demoModel,
      content: userPrompt,
      preamble: null,
    },
    {
      type: 'thinking',
      id: 'thinking-streaming',
      createdAt: thinkingStartedAt,
      model: demoModel,
      text: thinkingText,
      signature: null,
    },
    {
      type: 'thinking',
      id: 'thinking-done',
      createdAt: thinkingStartedAt,
      model: demoModel,
      text: thinkingText,
      signature: null,
    },
    shellTool as Block,
    ...agentReceiptMessages.slice(0, 2).map((message): Block => ({
      type: 'agent_message', id: message.id, turnId: 'turn-1',
      createdAt: message.timestamp, model: demoModel, message,
    })),
    {
      type: 'steer',
      id: 'steer-1',
      turnId: 'turn-1',
      createdAt: iso(100_000),
      model: demoModel,
      content: [
        {
          type: 'text',
          text: 'Also add a case for the expired cookie.'
        }
      ],
    },
    runningShellTool as Block,
    editingShellTool as Block,
    statusTool as Block,
    writeTool as Block,
    yieldTool as Block,
    abortTool as Block,
    errorTool as Block,
    {
      type: 'compaction_boundary',
      id: 'compaction-done',
      createdAt: iso(8_000),
      model: demoModel,
      summary: 'Kept the cookie helper and the new session assertion.',
      summaryTokens: 2400,
    },
    {
      type: 'text',
      id: 'assistant-1',
      createdAt: thinkingEndedAt,
      model: demoModel,
      text: assistantMarkdown,
    },
    {
      type: 'abort',
      id: 'abort-1',
      createdAt: iso(2_000),
      model: demoModel,
      isResumed: false,
    },
    {
      type: 'error',
      id: 'error-1',
      createdAt: iso(1_000),
      model: demoModel,
      message: 'Anthropic API request failed with HTTP 429: This request would exceed the rate limit of 50 requests per minute for your organization. Retry after 12 seconds.',
      code: 'rate_limit',
      diagnostics: {
        source: 'http',
        httpStatus: 429,
        providerCode: 'rate_limit_error',
        clientRequestId: 'req_01J8Y3Q6ZKX4',
      },
    },
    {
      type: 'user',
      id: 'user-attachments',
      turnId: 'turn-1',
      createdAt: iso(900),
      model: demoModel,
      content: [
        { type: 'image', source: { type: 'url', url: demoImageUrl } },
        {
          type: 'document',
          source: {
            data: new Uint8Array(),
            mediaType: 'application/pdf',
            fileName: 'login-failure.pdf'
          }
        },
        {
          type: 'reference',
          reference: encodeRemoteReference('zan-mbp', '/Users/zan/Projects/demi/package.json')
        },
        { type: 'text', text: 'Failing log and the screenshot from CI.' },
      ],
      preamble: null,
    },
  ]
}
