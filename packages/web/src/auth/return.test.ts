import { expect, test } from 'bun:test'
import { returnAddress, signInAddress } from './return'

// Cost: pure functions; under a millisecond.

test('a page opened signed out is kept through signing in', () => {
  const address = signInAddress('/chat/c-1?tab=files')
  expect(address).toBe('/login?next=%2Fchat%2Fc-1%3Ftab%3Dfiles')
  const query = Object.fromEntries(new URL(address, 'https://demi.test').searchParams)
  expect(returnAddress(query)).toBe('/chat/c-1?tab=files')
})

test('an ended session keeps the page it ended on beside its reason', () => {
  expect(signInAddress('/settings/account', 'expired')).toBe('/login?reason=expired&next=%2Fsettings%2Faccount')
  expect(signInAddress('/chat', 'expired')).toBe('/login?reason=expired')
})

test('signing in goes nowhere outside the app', () => {
  expect(returnAddress({ next: 'https://evil.test/' })).toBe('/chat')
  expect(returnAddress({ next: '//evil.test/' })).toBe('/chat')
  expect(returnAddress({ next: '/\\evil.test' })).toBe('/chat')
  expect(returnAddress({})).toBe('/chat')
})
