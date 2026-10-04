import { expect, test } from 'bun:test'
import { applyStyle, styleProblems } from '../ui-text'

const words = (text: string, style: Parameters<typeof styleProblems>[1], references: string[] = []) =>
  styleProblems(text, style, references).map((problem) => `${problem.word} → ${problem.want}`)

test('title style accepts the Apple Style Guide’s own examples', () => {
  for (const text of [
    'Skip This Backup',
    'Passwords Are Locked',
    'What to Do If Your iPhone Is Lost',
    'Start Up the Computer',
    'Turn On Apple Watch',
    'Log In to the Server',
    'High-Level Events',
    '64-Bit Addressing',
    'Built-in Camera',
    'How to Start Your Computer',
    'Export a Document as a PDF',
    'Collaborate with Others',
    'Restore iPhone from a Backup',
    'Go To…',
    'Go to Page…',
    'Don’t Save',
    'Add to Cart',
  ])
    expect(words(text, 'title')).toEqual([])
})

test('title style names each word to change, and the marks a title does not end with', () => {
  expect(words('Add source', 'title')).toEqual(['source → Source'])
  expect(words('Export A Document', 'title')).toEqual(['A → a'])
  expect(words('Sign in', 'title')).toEqual(['in → In'])
  expect(words('Command-line tool', 'title')).toEqual(['Command-line → Command-Line', 'tool → Tool'])
  expect(words('Open...', 'title')).toEqual(['... → …'])
  expect(words('Save.', 'title')).toEqual(['. → '])
  expect(words('Switch to claude-sonnet', 'title')).toEqual([])
  expect(words('Notify me when', 'title')).toEqual([])
})

test('a headline is a title as a fragment and a sentence as a sentence', () => {
  expect(words('Could not save the provider', 'headline')).toEqual(['not → Not', 'save → Save', 'provider → Provider'])
  expect(words('Could Not Save the Provider', 'headline')).toEqual([])
  expect(words('Remove this project?', 'headline')).toEqual([])
  expect(words('No Results', 'headline')).toEqual([])
})

test('sentence style capitalizes each sentence’s first word and names, and accepts named UI elements', () => {
  expect(words('Show scroll bars', 'sentence')).toEqual([])
  expect(words('Show Scroll Bars', 'sentence')).toEqual(['Scroll → scroll', 'Bars → bars'])
  expect(words('show scroll bars', 'sentence')).toEqual(['show → Show'])
  expect(words('Pairing code:', 'sentence')).toEqual([': → '])
  expect(words('Sign in to Claude Code. Then pick a model.', 'sentence')).toEqual([])
  expect(words('Turn off Reduce Transparency?', 'sentence', ['Reduce Transparency'])).toEqual([])
  expect(words('Press Escape to close it, or Shift-click the M mark.', 'sentence')).toEqual([])
  expect(words('name@example.com', 'placeholder')).toEqual([])
})

test('applying a style rewrites only the words it names', () => {
  expect(applyStyle('Add source...', 'title')).toBe('Add Source…')
  expect(applyStyle('Use As Primary Environment', 'title')).toBe('Use as Primary Environment')
  expect(applyStyle('Show Scroll Bars', 'sentence')).toBe('Show scroll bars')
})
