import type { DisplayedBlock as Block } from '@demicodes/web-ui/transport/protocol'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import { demoModel, thinkingText } from './blocks'

function ago(ms: number): string {
  return new Date(Date.now() - ms).toISOString()
}

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
): Block {
  return {
    type: 'tool_call',
    id,
    createdAt,
    model: demoModel,
    toolUseId: `${id}-use`,
    toolName: 'shell_exec',
    input: JSON.stringify({
      script: `echo ${description}`,
      description,
    }),
    status,
    streamingOutput: [],
    output: [],
    view: {
      chunks: [
        {
          stream: 'stdout',
          text: stdout,
        },
      ],
    },
  }
}

export function gallerySubagents(): SubagentRecord[] {
  return [
    {
      id: 'ag-cookie',
      name: 'Find the cookie assertion',
      phase: 'running',
      startedAt: ago(134_000),
      blocks: [
        user(
          'ag-cookie-user',
          ago(134_000),
          'Find the expect that still looks for sid in auth.test.ts. Do not edit yet.',
        ),
        thinking('ag-cookie-think', ago(120_000), thinkingText),
        shell(
          'ag-cookie-tool',
          ago(90_000),
          'Find the old cookie name in the login test',
          'packages/web/src/auth.test.ts:18:    expect(cookie.name).toBe("sid")\n',
          'executing',
        ),
      ],
    },
    {
      id: 'ag-logout',
      name: 'Check the logout path',
      phase: 'running',
      startedAt: ago(48_000),
      blocks: [
        user(
          'ag-logout-user',
          ago(48_000),
          'See whether logout still clears the renamed session cookie.',
        ),
        thinking(
          'ag-logout-think',
          ago(40_000),
          'Logout should drop the session cookie. The helper writes the new name; the test may still mention sid.',
        ),
      ],
    },
    {
      id: 'ag-snapshot',
      name: 'Snapshot strings',
      phase: 'running',
      startedAt: ago(12_000),
      blocks: [
        user(
          'ag-snapshot-user',
          ago(12_000),
          'List snapshot strings that still mention the old cookie name.',
        ),
      ],
    },
    {
      id: 'ag-comments',
      name: 'Update helper comments',
      phase: 'completed',
      startedAt: ago(420_000),
      endedAt: ago(240_000),
      blocks: [
        user(
          'ag-comments-user',
          ago(420_000),
          'Update comments on the cookie helper to say session, not sid. Leave the code alone.',
        ),
        thinking(
          'ag-comments-think',
          ago(400_000),
          'Comments only. The helper already writes the new header.',
        ),
        shell(
          'ag-comments-tool',
          ago(360_000),
          'Open the cookie helper',
          'packages/web/src/cookie.ts\n',
          'completed',
        ),
        text(
          'ag-comments-text',
          ago(240_000),
          'Comments now say `session`. The helper is unchanged.',
        ),
      ],
    },
    {
      id: 'ag-ci',
      name: 'CI screenshot review',
      phase: 'aborted',
      startedAt: ago(180_000),
      endedAt: ago(120_000),
      blocks: [
        user(
          'ag-ci-user',
          ago(180_000),
          'Describe what the CI screenshot shows about the login failure.',
        ),
        thinking(
          'ag-ci-think',
          ago(160_000),
          'The screenshot is an attachment on the parent turn.',
        ),
        {
          type: 'abort',
          id: 'ag-ci-abort',
          createdAt: ago(120_000),
          model: demoModel,
          isResumed: false,
        },
      ],
    },
  ]
}
