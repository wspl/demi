import { expect, test } from 'bun:test'
import type { Block, InstructionEntry, ModelSelection } from '@demicodes/protocol'
import { instructionRows, loadedInstructions } from '../instructions'

const model: ModelSelection = {
  providerId: 'test',
  model: { id: 'test-model', name: 'Test Model', contextWindow: 1000, outputLimit: null, thinking: [], acceptedExtensions: [] },
  thinking: null,
  serviceTierId: null,
}

let blockCount = 0

/** A context block of `source`, listing `instructions` as an instructions block does. */
function context(source: string, instructions?: InstructionEntry[]): Block {
  blockCount += 1
  const id = `context-${blockCount}`
  return { type: 'context', id, turnId: id, createdAt: '1970-01-01T00:00:00.000Z', model, source, text: '', instructions }
}

test('the card lists the newest instructions block, and nothing before the first or once nothing is left', () => {
  const root: InstructionEntry = { kind: 'file', path: '/repo/AGENTS.md', tokens: 10 }
  const web: InstructionEntry = { kind: 'file', path: '/repo/web/CLAUDE.md', tokens: 3 }
  expect(loadedInstructions([context('execution')])).toEqual([])
  const blocks = [context('instructions', [root]), context('execution'), context('instructions', [root, web])]
  expect(loadedInstructions(blocks)).toEqual([root, web])
  expect(loadedInstructions([...blocks, context('instructions')])).toEqual([])
})

test('each file is named from the outermost file’s directory, and a file too large has no estimate', () => {
  const rows = instructionRows([
    { kind: 'personal', tokens: 5 },
    { kind: 'file', path: '/repo/AGENTS.md', tokens: 10 },
    { kind: 'too_large', path: '/repo/web/AGENTS.md' },
  ])
  expect(rows.map(({ label, path, tokens }) => ({ label, path, tokens }))).toEqual([
    { label: 'Personal instructions', path: null, tokens: 5 },
    { label: 'AGENTS.md', path: '/repo/AGENTS.md', tokens: 10 },
    { label: 'web/AGENTS.md', path: '/repo/web/AGENTS.md', tokens: null },
  ])
})
