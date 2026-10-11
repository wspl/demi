import { expect, test } from 'bun:test'
import { limitStep, parseLimit, type LimitEvent, type LimitPhase, type LimitStep } from '../limit-field'

test('Unlimited asks first: Cancel keeps the number, the confirmation commits null', () => {
  const steps: [number | null, LimitPhase, LimitEvent, LimitStep][] = [
    [10, 'shown', 'chooseUnlimited', { phase: 'confirming' }],
    [10, 'confirming', 'cancel', { phase: 'shown' }],
    [10, 'confirming', 'confirm', { phase: 'shown', commit: null }],
    // Already Unlimited: nothing to ask.
    [null, 'shown', 'chooseUnlimited', { phase: 'shown' }],
    // Limited from Unlimited opens an empty field; left empty, it stays Unlimited.
    [null, 'shown', 'chooseLimited', { phase: 'typing' }],
    [null, 'typing', 'leaveEmpty', { phase: 'shown' }],
    [10, 'shown', 'chooseLimited', { phase: 'shown' }],
    // A confirmation that comes late commits nothing.
    [10, 'shown', 'confirm', { phase: 'shown' }],
  ]
  for (const [value, phase, event, step] of steps) {
    expect([value, phase, event, limitStep(value, phase, event)]).toEqual([value, phase, event, step])
  }
})

test('a limit is a whole number from 1', () => {
  const cases: [string, number | null][] = [
    ['1', 1],
    [' 12 ', 12],
    ['4294967295', 4_294_967_295],
    ['0', null],
    ['-1', null],
    ['1.5', null],
    ['1e3', null],
    ['', null],
    ['4294967296', null],
  ]
  for (const [text, limit] of cases) {
    expect([text, parseLimit(text)]).toEqual([text, limit])
  }
})
