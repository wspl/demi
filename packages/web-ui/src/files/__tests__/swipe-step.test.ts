import { expect, test } from 'bun:test'
import { startsSwipe, swipeAxis, swipeOffset, swipeStep, type StepReach } from '../swipe-step'

// The decisions behind the phone viewer's swipe, with no DOM: a few milliseconds.

const both: StepReach = { previous: true, next: true }
const first: StepReach = { previous: false, next: true }
const last: StepReach = { previous: true, next: false }
const width = 390

test('a finger swiped left shows the next picture and one swiped right the previous, as in Photos', () => {
  expect(swipeStep(-150, width, 300, both)).toBe(1)
  expect(swipeStep(150, width, 300, both)).toBe(-1)
})

test('a short, slow drag springs back, and a quick flick steps after a shorter one', () => {
  expect(swipeStep(-60, width, 600, both)).toBe(0)
  expect(swipeStep(-60, width, 80, both)).toBe(1)
  // A twitch is no flick, however fast.
  expect(swipeStep(-10, width, 5, both)).toBe(0)
})

test('at the first picture a swipe right and at the last a swipe left stops, however far it goes', () => {
  expect(swipeStep(300, width, 100, first)).toBe(0)
  expect(swipeStep(-300, width, 100, last)).toBe(0)
  expect(swipeStep(-300, width, 100, first)).toBe(1)
})

test('past the first or the last picture the picture follows only a little of the finger', () => {
  expect(swipeOffset(-100, both)).toBe(-100)
  expect(swipeOffset(100, first)).toBe(25)
  expect(swipeOffset(-100, last)).toBe(-25)
  expect(swipeOffset(-100, first)).toBe(-100)
})

test('only a picture at its fitted size is swiped by a finger; a zoomed one is moved by the drag, and a mouse always moves', () => {
  expect(startsSwipe('touch', true)).toBe(true)
  expect(startsSwipe('pen', true)).toBe(true)
  expect(startsSwipe('touch', false)).toBe(false)
  expect(startsSwipe('mouse', true)).toBe(false)
})

test('a drag is a swipe only when it goes mostly sideways once it left a click\'s reach', () => {
  expect(swipeAxis(2, 1)).toBe('click')
  expect(swipeAxis(-30, 10)).toBe('horizontal')
  expect(swipeAxis(10, 30)).toBe('vertical')
})
