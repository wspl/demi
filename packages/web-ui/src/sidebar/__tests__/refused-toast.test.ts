import { expect, test } from 'bun:test'
import { refusedText } from '../refused-toast'

// Cost: string building alone; milliseconds.

const RUNNING = 'A conversation with running work cannot be archived, restored or moved'

test('one refused conversation is named in the headline, and the reason reads as a sentence', () => {
  expect(refusedText('archive', [
    { title: 'Run `sleep 30` with your shell tool, then reply with the word done.', reasons: [RUNNING] },
  ])).toEqual({
    title: 'Could Not Archive “Run sleep 30 with…”',
    message: 'A conversation with running work cannot be archived, restored or moved.',
  })
  // A short title stays whole, without its closing period, and a reason that is a sentence keeps its own end.
  expect(refusedText('pin', [{ title: 'Fix login.', reasons: ['turn is running', 'Not now?'] }])).toEqual({
    title: 'Could Not Pin “Fix login”',
    message: 'Turn is running. Not now?',
  })
})

test('several refused conversations are counted, each with its reason on a line of its own', () => {
  expect(refusedText('archive', [
    { title: 'Run `sleep 30` with your shell tool, then reply with the word done.', reasons: [RUNNING] },
    { title: 'Fix login bug.', reasons: [RUNNING] },
  ])).toEqual({
    title: 'Could Not Archive 2 Conversations',
    message: [
      '“Run sleep 30 with…”: A conversation with running work cannot be archived, restored or moved.',
      '“Fix login bug”: A conversation with running work cannot be archived, restored or moved.',
    ].join('\n'),
  })
})
