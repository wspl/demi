import { expect, test } from 'bun:test'
import { directReason, type DirectAttempt, type DirectReason, type DirectStatus } from '../direct'

// The reason a device's page gives for going through the server, from what
// the last attempt saw (`direct-channel.md` § What the user sees). Pure;
// milliseconds.

const failed: DirectAttempt = {
  startedAt: '2026-10-08T09:00:00.000Z',
  durationMs: 10_000,
  outcome: 'failed',
  stage: 'checking',
  browser: { local: ['3f2a.local'], public: ['198.51.100.4'] },
  device: { local: ['192.168.1.20'], public: ['203.0.113.9'] },
  pairs: { tried: 4, answered: 0 },
  permission: 'granted',
}

const status: DirectStatus = {
  enabled: true,
  crossing: true,
  permission: 'granted',
  connected: false,
  trying: false,
  roundTripMs: null,
  attempt: failed,
  nextAt: null,
}

const cases: { scenario: string; status: DirectStatus; reason: DirectReason | null }[] = [
  { scenario: 'connected', status: { ...status, connected: true, attempt: { ...failed, outcome: 'connected', stage: null } }, reason: null },
  { scenario: 'switched off, even while a channel stood', status: { ...status, enabled: false, connected: true }, reason: { kind: 'off' } },
  { scenario: 'the browser blocks local network access', status: { ...status, permission: 'denied' }, reason: { kind: 'blocked' } },
  { scenario: 'an attempt stopped at the permission', status: { ...status, attempt: { ...failed, stage: 'permission' } }, reason: { kind: 'blocked' } },
  { scenario: 'no attempt yet', status: { ...status, attempt: null }, reason: { kind: 'notYet' } },
  { scenario: 'the runner was busy', status: { ...status, attempt: { ...failed, outcome: 'busy' } }, reason: { kind: 'busy' } },
  {
    scenario: 'a channel that stood dropped',
    status: { ...status, attempt: { ...failed, outcome: 'dropped', endedAt: '2026-10-08T09:30:00.000Z' } },
    reason: { kind: 'dropped', endedAt: '2026-10-08T09:30:00.000Z' },
  },
  { scenario: 'crossing networks is off', status: { ...status, crossing: false }, reason: { kind: 'notOffered' } },
  {
    scenario: 'the browser learned no public address',
    status: { ...status, attempt: { ...failed, browser: { local: ['3f2a.local'], public: [] } } },
    reason: { kind: 'network', side: 'browser' },
  },
  {
    scenario: 'the device learned no public address',
    status: { ...status, attempt: { ...failed, device: { local: ['192.168.1.20'], public: [] } } },
    reason: { kind: 'network', side: 'device' },
  },
  { scenario: 'both found their public address and no pair answered', status, reason: { kind: 'unreachable', pairs: 4 } },
]

for (const { scenario, status: given, reason } of cases) {
  test(`the reason when ${scenario}`, () => {
    expect(directReason(given)).toEqual(reason)
  })
}
