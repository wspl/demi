import { expect, test } from 'bun:test'
import { limitStep, parseLimit, type LimitEvent, type LimitPhase, type LimitValue } from '../limit-field'
test('Unlimited asks first and keeps the number, which Limited brings back as it was', () => {
  // The user lifts a limit of 7, cancels once, confirms, then chooses Limited again.
  const flow: [LimitEvent, LimitPhase, LimitValue][] = [
    ['chooseUnlimited', 'confirming', { limit: 7, unlimited: false }],
    ['cancel', 'shown', { limit: 7, unlimited: false }],
    ['chooseUnlimited', 'confirming', { limit: 7, unlimited: false }],
    ['confirm', 'shown', { limit: 7, unlimited: true }],
    // Already Unlimited: nothing to ask.
    ['chooseUnlimited', 'shown', { limit: 7, unlimited: true }],
    // A confirmation that comes late commits nothing.
    ['confirm', 'shown', { limit: 7, unlimited: true }],
    ['chooseLimited', 'shown', { limit: 7, unlimited: false }],
    // Already Limited: nothing changes.
    ['chooseLimited', 'shown', { limit: 7, unlimited: false }],
  ]
  let value: LimitValue = { limit: 7, unlimited: false }
  let phase: LimitPhase = 'shown'
  for (const [event, nextPhase, nextValue] of flow) {
    const step = limitStep(value, phase, event)
    phase = step.phase
    value = step.commit ?? value
    expect([event, phase, value]).toEqual([event, nextPhase, nextValue])
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
