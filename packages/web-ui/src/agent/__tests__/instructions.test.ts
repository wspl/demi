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
  const root: InstructionEntry = { kind: 'file', path: '/repo/AGENTS.md' }
  const web: InstructionEntry = { kind: 'file', path: '/repo/web/CLAUDE.md' }
  expect(loadedInstructions([context('execution')])).toEqual([])
  const blocks = [context('instructions', [root]), context('execution'), context('instructions', [root, web])]
  expect(loadedInstructions(blocks)).toEqual([root, web])
  expect(loadedInstructions([...blocks, context('instructions')])).toEqual([])
})

test('each file is named from the outermost file’s directory, and a file too large is marked', () => {
  const rows = instructionRows([
    { kind: 'personal' },
    { kind: 'file', path: '/repo/AGENTS.md' },
    { kind: 'too_large', path: '/repo/web/AGENTS.md' },
  ])
  expect(rows.map(({ label, path, tooLarge }) => ({ label, path, tooLarge }))).toEqual([
    { label: 'Personal instructions', path: null, tooLarge: false },
    { label: 'AGENTS.md', path: '/repo/AGENTS.md', tooLarge: false },
    { label: 'web/AGENTS.md', path: '/repo/web/AGENTS.md', tooLarge: true },
  ])
})
