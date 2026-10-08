import { expect, test } from 'bun:test'
import {
  directReason,
  pathKind,
  pathSentence,
  pathsLatency,
  shownAddress,
  statusPath,
  type DirectAttempt,
  type DirectReason,
  type DirectStatus,
  type PathKind,
} from '../direct'

// The reason a device's page gives for going via the relay, from its
// route, its peer and what the last attempt saw (`direct-channel.md`
// § What the user sees), the kind of path the address in use names, and
// the Latency row that compares the two paths. Pure; milliseconds.

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
  measuring: false,
}

/** A peer standing on the device's address `address`. */
function connected(address: string): DirectStatus {
  return { ...status, peer: true, chosen: true, attempt: { ...failed, outcome: 'connected', stage: 'connected', inUse: { address, port: 51820 } } }
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

const latencies: { scenario: string; status: DirectStatus; latency: string | null }[] = [
  {
    scenario: 'both paths measured, the peer on the local network',
    status: { ...connected('192.168.1.20'), figures: { direct: { latencyMs: 1.8, loss: 0 }, relay: { latencyMs: 480.4, loss: null } } },
    latency: 'LAN 2 ms · Relay 480 ms',
  },
  {
    scenario: 'a peer across the internet loses probes',
    status: { ...connected('203.0.113.9'), figures: { direct: { latencyMs: 620, loss: 0.1 }, relay } },
    latency: 'P2P 620 ms, 10% lost · Relay 30 ms',
  },
  {
    scenario: 'a peer on this computer loses one probe in twenty',
    status: { ...connected('127.0.0.1'), figures: { direct: { latencyMs: 0.42, loss: 0.05 }, relay } },
    latency: 'This Computer 0.4 ms, 5% lost · Relay 30 ms',
  },
  {
    scenario: 'there is no peer',
    status: { ...status, figures: { direct: null, relay: { latencyMs: 480, loss: null } } },
    latency: 'Relay 480 ms',
  },
  { scenario: 'nothing is measured yet', status, latency: null },
]

for (const { scenario, status: given, latency } of latencies) {
  test(`the Latency row when ${scenario}`, () => {
    expect(pathsLatency(given)).toBe(latency)
  })
}

const kinds: { address: string; kind: PathKind }[] = [
  { address: '127.0.0.1', kind: 'thisComputer' },
  { address: '::1', kind: 'thisComputer' },
  { address: '10.0.0.7', kind: 'localNetwork' },
  { address: '172.16.4.2', kind: 'localNetwork' },
  { address: '172.32.0.1', kind: 'internet' },
  { address: '192.168.1.20', kind: 'localNetwork' },
  { address: '169.254.10.1', kind: 'localNetwork' },
  { address: '100.64.0.1', kind: 'localNetwork' },
  { address: '100.127.255.254', kind: 'localNetwork' },
  { address: '100.128.0.1', kind: 'internet' },
  { address: '203.0.113.9', kind: 'internet' },
  { address: 'fd7a:115c:a1e0::1', kind: 'localNetwork' },
  { address: 'fe80::1c2d:3e4f', kind: 'localNetwork' },
  { address: '2001:db8::5', kind: 'internet' },
  { address: '::ffff:192.168.1.20', kind: 'localNetwork' },
  { address: '::ffff:203.0.113.9', kind: 'internet' },
]

for (const { address, kind } of kinds) {
  test(`${address} is a path of kind ${kind}`, () => {
    expect(pathKind(address)).toBe(kind)
  })
}

test('the path in use is the peer’s kind while the choice uses it, the relay otherwise', () => {
  expect(statusPath(connected('100.101.102.103'))).toBe('localNetwork')
  expect(statusPath({ ...connected('100.101.102.103'), chosen: false })).toBe('relay')
  expect(statusPath(status)).toBe('relay')
})

test('the P2P row names the kind and the address in use', () => {
  expect(pathSentence('192.168.1.20')).toBe('LAN · 192.168.1.20')
  expect(pathSentence('127.0.0.1')).toBe('This Computer · 127.0.0.1')
})
