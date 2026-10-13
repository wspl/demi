import { expect, test } from 'bun:test'
import { messageBlockEnds } from '../render'

// Which blocks of a streaming reply settle (`file-previews.md`, a reply that
// streams renders a block at a time): a settled block renders once, so one
// that later text would change shows wrong until the reply ends.
// Cost: pure, about 200 ms for the streamed replies.

/** The settled blocks of `text`, each with the blank lines after it. */
function settled(text: string): string[] {
  const ends = messageBlockEnds(text)
  return ends.map((end, index) => text.slice(ends[index - 1] ?? 0, end))
}

test('a blank line ends a block outside a fence, a list item, an indented code block and display math', () => {
  expect(settled('One paragraph.\n\nAnother.\n\nA third is arr')).toEqual(['One paragraph.\n\n'])
  expect(settled('```ts\na\n\nb\n```\n\nafter\n')).toEqual(['```ts\na\n\nb\n```\n\n'])
  expect(settled('```ts\na\n\nb\n\nstill code\n')).toEqual([])
  expect(settled('- a\n\n  continued\n\n- b\n\nafter\n')).toEqual(['- a\n\n  continued\n\n- b\n\n'])
  expect(settled('    a\n\n    b\n\nafter\n')).toEqual(['    a\n\n    b\n\n'])
  expect(settled('| a | b |\n|---|---|\n| c | d |\n\nafter\n')).toEqual(['| a | b |\n|---|---|\n| c | d |\n\n'])
  expect(settled('$$\nx^2\n$$\n\ny\n')).toEqual(['$$\nx^2\n$$\n\n'])
  expect(settled('# Heading\nIts paragraph\n')).toEqual(['# Heading\n'])
})

test('a block stays live while what follows could still join it or change it', () => {
  // The next item's number has arrived without its dot: it may continue the list.
  expect(settled('1. a\n\n2')).toEqual([])
  expect(settled('1. a\n\n2. b\n\nnext\n')).toEqual(['1. a\n\n2. b\n\n'])
  // Display math lexes as text until its closing line arrives.
  expect(settled('$$\nx^2\n\ny\n\nz\n')).toEqual([])
  // Only the whole text nests the body inside `<details>`.
  expect(settled('Intro.\n\n<details>\n<summary>More</summary>\n\nbody\n\nend\n')).toEqual(['Intro.\n\n'])
})

/** A deterministic source of numbers in [0, 1), so a failing case is found again. */
function seeded(seed: number): () => number {
  let state = seed
  return () => {
    state = (state + 0x6d2b79f5) | 0
    let t = Math.imul(state ^ (state >>> 15), 1 | state)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** A reply of the blocks an agent writes, blank lines inside lists, fences and code included. */
function randomReply(random: () => number): string {
  const pick = <T>(items: readonly T[]): T => items[Math.floor(random() * items.length)]!
  const words = () => Array.from({ length: 2 + Math.floor(random() * 8) }, () => pick(['the', 'step', '**bold**', '`code`', 'x', '2.', '-', '1)', '$', '|'])).join(' ')
  const blocks = [
    () => words(),
    () => `${words()}\n${words()}`,
    () => `# ${words()}`,
    () => `> ${words()}\n> ${words()}`,
    () => `1. ${words()}\n\n2. ${words()}\n\n   ${words()}\n\n3. ${words()}`,
    () => `- ${words()}\n\n  ${words()}\n- ${words()}`,
    () => `\`\`\`ts\n${words()}\n\n${words()}\n\`\`\``,
    () => `    ${words()}\n\n    ${words()}`,
    () => `| a | b |\n|---|---|\n| ${words()} | x |`,
    () => `$$\n${words()}\n$$`,
    () => '---',
    // Lines that make the lines above them a heading or a table's head.
    () => `${words()}\n---`,
    () => `${words()}\nx | y\n|---|---|\n| a | b |`,
  ]
  return Array.from({ length: 2 + Math.floor(random() * 8) }, () => pick(blocks)()).join(pick(['\n\n', '\n\n\n', '\n']))
}

// A block settled at any moment of a reply's stream is a block of the reply
// as it ends, with the same text: what arrived later never changed it.
test('a block settled while a reply streams is a block of the whole reply', () => {
  const random = seeded(3)
  for (let sample = 0; sample < 300; sample += 1) {
    const reply = `${randomReply(random)}\n\nThe end.\n`
    const whole = new Set(messageBlockEnds(reply))
    for (let length = 1; length < reply.length; length += 1 + Math.floor(random() * 4)) {
      const prefix = reply.slice(0, length)
      for (const end of messageBlockEnds(prefix))
        expect({ reply, prefix, end, settled: whole.has(end) }).toEqual({ reply, prefix, end, settled: true })
    }
  }
})
