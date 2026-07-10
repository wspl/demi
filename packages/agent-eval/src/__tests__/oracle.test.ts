import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { runOracles } from '../oracle'
import { snapshotWorkspace } from '../workspace'

const model: ModelSelection = {
  providerId: 'stub',
  model: { id: 'm', name: 'm', contextWindow: 1000, inputLimit: null, thinking: [], acceptedExtensions: [] },
  thinking: null,
}

function textBlock(text: string): Block {
  return { type: 'text', id: crypto.randomUUID(), createdAt: '2026-01-01T00:00:00.000Z', model, text }
}

function toolCallBlock(toolName: string): Block {
  return {
    type: 'tool_call',
    id: crypto.randomUUID(),
    createdAt: '2026-01-01T00:00:00.000Z',
    model,
    toolUseId: crypto.randomUUID(),
    toolName,
    input: '{}',
    status: 'completed',
    streamingOutput: [],
    output: [{ type: 'text', text: 'ok' }],
    view: null,
  }
}

async function context(workspace: string, blocks: Block[] = []) {
  const snapshot = await snapshotWorkspace(workspace)
  return { workspace, transcriptBlocks: blocks, workspaceBefore: snapshot, workspaceAfter: snapshot }
}

test('command oracle checks exit code and stream content', async () => {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-oracle-'))
  const pass = await runOracles(
    [{ type: 'command', name: 'echo', command: ['echo', 'hello world'], timeoutMs: 5_000, expectedExitCode: 0, stdoutIncludes: ['hello'] }],
    await context(workspace),
  )
  expect(pass[0]).toMatchObject({ passed: true })

  const fail = await runOracles(
    [
      {
        type: 'command',
        name: 'false',
        command: ['sh', '-c', 'echo bad >&2; exit 3'],
        timeoutMs: 5_000,
        expectedExitCode: 0,
        stderrExcludes: ['bad'],
      },
    ],
    await context(workspace),
  )
  expect(fail[0]).toMatchObject({ passed: false })
  expect(fail[0]!.summary).toContain('exit code 3')
  expect(fail[0]!.summary).toContain('stderr contains')
})

test('file oracle checks existence and content', async () => {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-oracle-file-'))
  await writeFile(join(workspace, 'done.txt'), 'all done\n')
  const results = await runOracles(
    [
      { type: 'file', path: 'done.txt', mustExist: true, textIncludes: ['done'] },
      { type: 'file', path: 'missing.txt', mustExist: true },
      { type: 'file', path: 'done.txt', mustExist: true, textIncludes: ['absent-token'] },
    ],
    await context(workspace),
  )
  expect(results.map((result) => result.passed)).toEqual([true, false, false])
})

test('transcript oracle checks tool calls and assistant text', async () => {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-oracle-transcript-'))
  const blocks = [toolCallBlock('shell_exec'), textBlock('the file is created')]
  const results = await runOracles(
    [
      {
        type: 'transcript',
        assertions: [
          { kind: 'tool_call', toolName: 'shell_exec', minCount: 1 },
          { kind: 'assistant_text_includes', text: 'created' },
          { kind: 'steer', minCount: 1 },
        ],
      },
    ],
    await context(workspace, blocks),
  )
  expect(results[0]).toMatchObject({ passed: false })
  expect(results[0]!.summary).toContain('steers')
})

test('diff oracle enforces allowed and forbidden paths', async () => {
  const workspace = await mkdtemp(join(tmpdir(), 'demi-oracle-diff-'))
  await writeFile(join(workspace, 'keep.txt'), 'original')
  const before = await snapshotWorkspace(workspace)
  await writeFile(join(workspace, 'keep.txt'), 'changed')
  await writeFile(join(workspace, 'new.log'), 'log')
  const after = await snapshotWorkspace(workspace)

  const results = await runOracles(
    [
      { type: 'diff', allowedPaths: ['keep.txt'] },
      { type: 'diff', forbiddenPaths: ['**/*.log'] },
      { type: 'diff', allowedPaths: ['keep.txt', '*.log'] },
    ],
    { workspace, transcriptBlocks: [], workspaceBefore: before, workspaceAfter: after },
  )
  expect(results[0]).toMatchObject({ passed: false })
  expect(results[0]!.summary).toContain('new.log')
  expect(results[1]).toMatchObject({ passed: false })
  expect(results[2]).toMatchObject({ passed: true })
})
