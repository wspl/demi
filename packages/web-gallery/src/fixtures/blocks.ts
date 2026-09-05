import type { Block, ModelSelection, TokenUsage, UserContentBlock } from '@demicodes/core'
import type { MessageListBlock } from '@demicodes/web-ui/agent/pending-steers'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'

export const demoModel: ModelSelection = {
  providerId: 'anthropic',
  model: {
    id: 'demo-model',
    name: 'Demo model',
    contextWindow: 200_000,
    inputLimit: 180_000,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

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

function toolCall(partial: Pick<ToolCallBlock, 'id' | 'toolName' | 'input' | 'status'> & Partial<ToolCallBlock>): ToolCallBlock {
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
  { type: 'text', text: 'The login test in `packages/web/src/auth.test.ts` is failing after the session cookie rename. Keep the fix in that file.' },
]

export const steerPrompt: UserContentBlock[] = [
  { type: 'text', text: 'Do not touch the cookie helper. Only fix the assertion.' },
]

export const assistantMarkdown = `The cookie helper is fine. The test still expects \`sid\`.

I updated the assertion in \`auth.test.ts\` and left \`cookie.ts\` alone.

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
  view: {
    chunks: [
      { stream: 'stdout', text: 'packages/web/src/auth.test.ts:18:    expect(cookie.name).toBe("sid")\n' },
    ],
  },
})

export const runningShellTool = toolCall({
  id: 'tool-shell-run',
  toolName: 'shell_exec',
  status: 'executing',
  input: JSON.stringify({
    script: 'bun test packages/web/src/auth.test.ts',
    description: 'Run the login test',
  }),
  view: {
    chunks: [{ stream: 'stdout', text: 'bun test v1.2\n' }],
  },
})

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
  input: JSON.stringify({ commandId: 'cmd_1', description: 'Check long-running command' }),
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

export function transcriptDemoBlocks(): MessageListBlock[] {
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
    {
      type: 'steer',
      id: 'steer-1',
      turnId: 'turn-1',
      createdAt: iso(100_000),
      model: demoModel,
      content: [{ type: 'text', text: 'Also add a case for the expired cookie.' }],
    },
    runningShellTool as Block,
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
      type: 'pending_steer',
      id: 'pending-steer-1',
      pendingSteerId: 'pending-1',
      content: steerPrompt,
    },
    {
      type: 'user',
      id: 'user-attachments',
      turnId: 'turn-1',
      createdAt: iso(900),
      model: demoModel,
      content: [
        { type: 'image', source: { type: 'url', url: demoImageUrl } },
        { type: 'document', source: { data: new Uint8Array(), mediaType: 'application/pdf', fileName: 'login-failure.pdf' } },
        { type: 'text', text: 'Failing log and the screenshot from CI.' },
      ],
      preamble: null,
    },
  ]
}
