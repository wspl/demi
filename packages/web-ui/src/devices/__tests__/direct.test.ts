import { expect, test } from 'bun:test'
import { directReason, pathsFootnote, shownAddress, type DirectAttempt, type DirectReason, type DirectStatus } from '../direct'

// The reason a device's page gives for going via the relay, from its
// route, its peer and what the last attempt saw (`direct-channel.md`
// § What the user sees), and the footnote that compares the two paths.
// Pure; milliseconds.

const failed: DirectAttempt = {
  startedAt: '2026-10-08T09:00:00.000Z',
  durationMs: 10_000,
  outcome: 'failed',
  stage: 'checking',
  browser: { local: ['3f2a.local'], public: ['198.51.100.4'] },
  device: { local: ['192.168.1.20'], public: ['203.0.113.9'] },
  pairs: { tried: 4, answered: 0 },
  inUse: null,
  permission: 'granted',
}

const status: DirectStatus = {
  route: 'automatic',
  crossing: true,
  permission: 'granted',
  peer: false,
  chosen: false,
  trying: false,
  attempt: failed,
  nextAt: null,
  figures: { direct: null, relay: null },
}

const slow = { latencyMs: 90, loss: 0 }
const relay = { latencyMs: 30, loss: null }

const cases: { scenario: string; status: DirectStatus; reason: DirectReason | null }[] = [
  {
    scenario: 'connected via P2P',
    status: { ...status, peer: true, chosen: true, attempt: { ...failed, outcome: 'connected', stage: 'connected' } },
    reason: null,
  },
  { scenario: 'Relay Only, even with a peer', status: { ...status, route: 'server', peer: true, chosen: true }, reason: { kind: 'serverOnly' } },
  {
    scenario: 'a peer stands but Automatic finds it slower',
    status: { ...status, peer: true, figures: { direct: slow, relay } },
    reason: { kind: 'slower' },
  },
  { scenario: 'the browser blocks local network access', status: { ...status, permission: 'denied' }, reason: { kind: 'blocked' } },
  { scenario: 'an attempt stopped at the permission', status: { ...status, attempt: { ...failed, stage: 'permission' } }, reason: { kind: 'blocked' } },
  { scenario: 'no attempt yet', status: { ...status, attempt: null }, reason: { kind: 'notYet' } },
  { scenario: 'the runner was busy', status: { ...status, attempt: { ...failed, outcome: 'busy' } }, reason: { kind: 'busy' } },
  {
    scenario: 'a channel that stood dropped',
    status: { ...status, attempt: { ...failed, outcome: 'dropped' } },
    reason: { kind: 'dropped' },
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
  { scenario: 'both found their public address and no pair answered', status, reason: { kind: 'unreachable' } },
]

for (const { scenario, status: given, reason } of cases) {
  test(`the reason when ${scenario}`, () => {
    expect(directReason(given)).toEqual(reason)
  })
}

test('a browser’s random .local name for its own address shows as hidden; a real address as it is', () => {
  expect(shownAddress('7c1e4a52-9b0d-4c1e-8e3e-1a2b3c4d5e6f.local')).toBe('Hidden by the browser')
  expect(shownAddress('192.168.1.20')).toBe('192.168.1.20')
})

const footnotes: { scenario: string; status: DirectStatus; footnote: string | null }[] = [
  {
    scenario: 'both paths measured',
    status: { ...status, peer: true, figures: { direct: { latencyMs: 1.8, loss: 0 }, relay: { latencyMs: 480.4, loss: null } } },
    footnote: 'P2P 2 ms · Relay 480 ms',
  },
  {
    scenario: 'the direct path loses probes',
    status: { ...status, peer: true, figures: { direct: { latencyMs: 620, loss: 0.06 }, relay } },
    footnote: 'P2P 620 ms, 6% lost · Relay 30 ms',
  },
  {
    scenario: 'a direct path on a local network loses one probe in two hundred',
    status: { ...status, peer: true, figures: { direct: { latencyMs: 0.42, loss: 0.005 }, relay } },
    footnote: 'P2P 0.4 ms, 0.5% lost · Relay 30 ms',
  },
  {
    scenario: 'there is no peer',
    status: { ...status, figures: { direct: null, relay: { latencyMs: 480, loss: null } } },
    footnote: 'Relay 480 ms',
  },
  { scenario: 'nothing is measured yet', status, footnote: null },
]

for (const { scenario, status: given, footnote } of footnotes) {
  test(`the footnote when ${scenario}`, () => {
    expect(pathsFootnote(given)).toBe(footnote)
  })
}
