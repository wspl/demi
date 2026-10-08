import { expect, test } from 'bun:test'
import type { DeviceRoute } from '@demicodes/web-ui/devices/direct'
import type { MeasureClock } from './measure'
import { DeviceMeter } from './meter'

// The measuring of a device's paths (`direct-channel.md` § Measuring the
// paths): nothing is probed until something asks, a measurement sends 20
// probes on each path 100 ms apart and then stops, and Automatic decides
// from it. Paths and a clock the test plays; no real time passes.

/** A clock whose timers run as the test moves it on. */
class Clock implements MeasureClock {
  private time = 0
  private timers: { at: number; run: () => void }[] = []

  now(): number {
    return this.time
  }

  after(ms: number, run: () => void): () => void {
    const timer = { at: this.time + ms, run }
    this.timers.push(timer)
    return () => {
      this.timers = this.timers.filter((other) => other !== timer)
    }
  }

  /** Moves the clock on by `ms`, running each timer due on the way in order. */
  advance(ms: number): void {
    const end = this.time + ms
    for (;;) {
      const next = this.timers.filter((timer) => timer.at <= end).sort((a, b) => a.at - b.at)[0]
      if (!next)
        break
      this.timers = this.timers.filter((timer) => timer !== next)
      this.time = next.at
      next.run()
    }
    this.time = end
  }
}

/** One path: when each probe went, and its answer after `rtt(index)` ms, none for null. */
function path(clock: Clock, rtt: (index: number) => number | null) {
  const sentAt: number[] = []
  const listeners = new Set<(id: number) => void>()
  return {
    sentAt,
    send(id: number) {
      const delay = rtt(sentAt.length)
      sentAt.push(clock.now())
      if (delay !== null)
        clock.after(delay, () => listeners.forEach((listener) => listener(id)))
    },
    answers(listener: (id: number) => void) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
  }
}

/** A device whose relay answers in 30 ms and whose peer, when `peer` says so, as `direct` says. */
function device(options: { direct?: (index: number) => number | null; route?: DeviceRoute; peerLater?: boolean } = {}) {
  const clock = new Clock()
  const relay = path(clock, () => 30)
  const direct = options.direct ? path(clock, options.direct) : null
  const standing = direct
    ? { probe: (id: number) => direct.send(id), onProbe: (listener: (id: number) => void) => direct.answers(listener) }
    : null
  /** The peer connected now; with `peerLater`, none until `connect`. */
  let peer = options.peerLater ? null : standing
  const slower: boolean[] = []
  const meter = new DeviceMeter(
    {
      connected: () => peer,
      setSlower: (value) => void slower.push(value),
    },
    { ping: (id) => relay.send(id), onPong: (listener) => relay.answers(listener) },
    () => options.route ?? 'automatic',
    undefined,
    clock,
  )
  return { clock, relay, direct, meter, slower, connect: () => void (peer = standing) }
}

test('nothing is probed until something asks for a measurement', () => {
  const { clock, relay, direct } = device({ direct: () => 2 })
  clock.advance(60_000)
  expect(relay.sentAt).toEqual([])
  expect(direct!.sentAt).toEqual([])
})

test('a measurement sends 20 probes on each path, 100 ms apart, and then none', async () => {
  const { clock, relay, direct, meter } = device({ direct: () => 2 })
  const measured = meter.measure()
  clock.advance(60_000)
  await measured
  const expected = Array.from({ length: 20 }, (_, index) => index * 100)
  expect(relay.sentAt).toEqual(expected)
  expect(direct!.sentAt).toEqual(expected)
  expect(meter.state.figures).toEqual({ direct: { latencyMs: 2, loss: 0 }, relay: { latencyMs: 30, loss: null } })
})

test('the measuring flag is set as the measurement starts and cleared as it ends, the figures shown meanwhile staying', async () => {
  const { clock, meter } = device({ direct: () => 2 })
  const first = meter.measure()
  expect(meter.state.measuring).toBe(true)
  clock.advance(5_000)
  await first
  expect(meter.state.measuring).toBe(false)
  const shown = meter.state.figures
  const second = meter.measure()
  expect(meter.state.measuring).toBe(true)
  expect(meter.state.figures).toBe(shown)
  clock.advance(5_000)
  await second
  expect(meter.state.measuring).toBe(false)
})

test('a measurement asked for while one runs joins it', async () => {
  const { clock, relay, meter } = device({ direct: () => 2 })
  const first = meter.measure()
  clock.advance(500)
  const joined = meter.measure()
  clock.advance(60_000)
  await Promise.all([first, joined])
  expect(relay.sentAt).toHaveLength(20)
})

test('a peer that connects while a measurement without it runs is measured once that one ends', async () => {
  const { clock, relay, direct, meter, connect } = device({ direct: () => 2, peerLater: true })
  const first = meter.measure()
  clock.advance(500)
  connect()
  const second = meter.measure()
  // The first measurement's last probe comes back at 1930 ms.
  clock.advance(1_430)
  await first
  clock.advance(60_000)
  await second
  expect(relay.sentAt).toHaveLength(40)
  // The peer's probes go once the first measurement ended.
  expect(direct!.sentAt).toHaveLength(20)
  expect(direct!.sentAt[0]).toBe(1_930)
  expect(meter.state.figures.direct).toEqual({ latencyMs: 2, loss: 0 })
})

test('a probe unanswered within 2 seconds counts as lost, even when its answer comes later', async () => {
  const { clock, meter } = device({ direct: (index) => (index === 3 ? 2_500 : index === 7 ? null : 2) })
  const measured = meter.measure()
  clock.advance(60_000)
  await measured
  expect(meter.state.figures.direct).toEqual({ latencyMs: 2, loss: 0.1 })
})

const choices: { scenario: string; direct: (index: number) => number | null; slower: boolean }[] = [
  { scenario: 'P2P loses one of its probes', direct: (index) => (index === 5 ? null : 2), slower: false },
  { scenario: 'P2P loses two of its probes', direct: (index) => (index === 5 || index === 9 ? null : 2), slower: true },
  { scenario: 'P2P is 20 ms above the relay', direct: () => 50, slower: false },
  { scenario: 'P2P is more than 20 ms above the relay', direct: () => 51, slower: true },
]

for (const { scenario, direct, slower } of choices) {
  test(`Automatic ${slower ? 'moves to the relay' : 'keeps P2P'} when ${scenario}`, async () => {
    const device_ = device({ direct })
    const measured = device_.meter.measure()
    device_.clock.advance(60_000)
    await measured
    expect(device_.slower).toEqual([slower])
  })
}

test('without a peer only the relay is probed, and the choice is left alone', async () => {
  const { clock, relay, meter, slower } = device()
  const measured = meter.measure()
  clock.advance(60_000)
  await measured
  expect(relay.sentAt).toHaveLength(20)
  expect(meter.state.figures).toEqual({ direct: null, relay: { latencyMs: 30, loss: null } })
  expect(slower).toEqual([])
})

test('a route other than Automatic is not decided by the measurement', async () => {
  const { clock, meter, slower } = device({ direct: () => 80, route: 'direct' })
  const measured = meter.measure()
  clock.advance(60_000)
  await measured
  expect(meter.state.figures.direct?.latencyMs).toBe(80)
  expect(slower).toEqual([])
})

test('closing the device ends its measurement and sends nothing more', async () => {
  const { clock, relay, meter } = device({ direct: () => 2 })
  const measured = meter.measure()
  clock.advance(450)
  meter.close()
  await measured
  clock.advance(60_000)
  expect(relay.sentAt).toHaveLength(5)
  expect(meter.state.measuring).toBe(false)
})
