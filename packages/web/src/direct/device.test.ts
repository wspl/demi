import { expect, test } from 'bun:test'
import { DeviceDirect, RETRY_MS, type Choice } from './device'
import type { DirectAttempt } from '@demicodes/web-ui/devices/direct'
import type { DirectPeer } from './peer'

// The page's choice of path to a device (`direct-channel.md` § Choosing the
// path), with peers and a clock the test plays: direct while a peer is
// connected, back to the relay at once when it fails, and the attempts
// again, at once for each reason and otherwise after the waits.

/** A peer the test connects or fails, and closes. */
class Peer implements DirectPeer {
  readonly ended = Promise.withResolvers<void>()
  readonly closed = this.ended.promise
  readonly attempt: DirectAttempt = {
    startedAt: '2026-10-08T09:00:00.000Z',
    durationMs: 20,
    outcome: 'connected',
    stage: null,
    browser: { local: ['3f2a.local'], public: [] },
    device: { local: ['127.0.0.1'], public: [] },
    pairs: { tried: 1, answered: 1 },
    inUse: null,
    permission: null,
  }
  probe(): void {}
  onProbe(): () => void {
    return () => {}
  }
  open(): never {
    throw new Error('the choice opens no channel')
  }
  close(): void {
    this.ended.resolve()
  }
}

/** A device whose attempts and waits the test answers. */
function device() {
  const attempts: PromiseWithResolvers<DirectPeer>[] = []
  const timers: { ms: number; run: () => void; cancelled: boolean }[] = []
  const direct = new DeviceDirect({
    ready: () => true,
    connect: () => {
      const attempt = Promise.withResolvers<DirectPeer>()
      attempts.push(attempt)
      return attempt.promise
    },
    after: (ms, run) => {
      const timer = { ms, run, cancelled: false }
      timers.push(timer)
      return () => {
        timer.cancelled = true
      }
    },
  })
  const choices: Choice[] = []
  direct.onChange((choice) => choices.push(choice))
  /** The wait now running, which the test lets pass. */
  const waiting = () => timers.filter((timer) => !timer.cancelled).at(-1)
  return { direct, attempts, timers, choices, waiting }
}

/** Lets the promises due run. */
async function flush(): Promise<void> {
  for (let turn = 0; turn < 5; turn++)
    await Promise.resolve()
}

test('the choice is direct while a peer is connected and the relay at once when it goes', async () => {
  const { direct, attempts, choices } = device()
  expect(direct.choice).toBe('relay')
  direct.tryNow()
  const peer = new Peer()
  attempts[0]!.resolve(peer)
  await flush()
  expect(direct.choice).toBe('direct')
  expect(direct.current()).toBe(peer)

  peer.close()
  await flush()
  expect(direct.choice).toBe('relay')
  expect(direct.current()).toBeNull()
  expect(choices).toEqual(['direct', 'relay'])
})

test('a channel that fails sets the relay at once and closes the peer', async () => {
  const { direct, attempts, choices } = device()
  direct.tryNow()
  const peer = new Peer()
  attempts[0]!.resolve(peer)
  await flush()
  direct.failed()
  expect(direct.choice).toBe('relay')
  expect(choices).toEqual(['direct', 'relay'])
  let closed = false
  void peer.closed.then(() => {
    closed = true
  })
  await flush()
  expect(closed).toBe(true)
})

test('attempts that fail wait 1, 2 and 5 minutes and then every 10, and a reason tries at once', async () => {
  const { direct, attempts, waiting } = device()
  direct.tryNow()
  for (const [index, ms] of [RETRY_MS[0], RETRY_MS[1], RETRY_MS[2], RETRY_MS[3], RETRY_MS[3]].entries()) {
    attempts[index]!.reject(new Error('no connection'))
    await flush()
    expect(waiting()?.ms).toBe(ms)
    waiting()!.run()
  }
  expect(attempts).toHaveLength(6)
  // One attempt runs at a time: a reason while one runs adds none.
  direct.tryNow()
  expect(attempts).toHaveLength(6)
  attempts[5]!.reject(new Error('no connection'))
  await flush()
  // A reason to try again, such as the runner connecting again, tries at
  // once and starts the waits over.
  direct.tryNow()
  expect(attempts).toHaveLength(7)
  attempts[6]!.reject(new Error('no connection'))
  await flush()
  expect(waiting()?.ms).toBe(RETRY_MS[0])
})

test('a browser that blocks local network access gets no attempt until the permission changes', async () => {
  const { direct, attempts, waiting } = device()
  direct.setPermission('denied')
  expect(direct.state.blocked).toBe(true)
  direct.tryNow()
  expect(attempts).toHaveLength(0)
  expect(waiting()).toBeUndefined()

  direct.setPermission('granted')
  expect(direct.state.blocked).toBe(false)
  expect(attempts).toHaveLength(1)
  attempts[0]!.resolve(new Peer())
  await flush()
  expect(direct.choice).toBe('direct')
})

test('a stopped device closes its peer, tells its streams, and tries no more', async () => {
  const { direct, attempts, choices, waiting } = device()
  direct.tryNow()
  const peer = new Peer()
  attempts[0]!.resolve(peer)
  await flush()
  direct.stop()
  expect(choices).toEqual(['direct', 'relay'])
  await flush()
  expect(waiting()).toBeUndefined()
  direct.tryNow()
  expect(attempts).toHaveLength(1)
})

test('no attempt starts before the signaling socket is open, whose opening is the reason to try', async () => {
  let open = false
  const attempts: PromiseWithResolvers<DirectPeer>[] = []
  const direct = new DeviceDirect({
    ready: () => open,
    connect: () => {
      const attempt = Promise.withResolvers<DirectPeer>()
      attempts.push(attempt)
      return attempt.promise
    },
    after: () => () => {},
  })
  direct.setPermission('prompt')
  direct.tryNow()
  expect(attempts).toHaveLength(0)
  open = true
  direct.tryNow()
  expect(attempts).toHaveLength(1)
})

test('a peer the backend closed is made again at once, in place of an attempt with the old introduction', async () => {
  const { direct, attempts, choices, waiting } = device()
  direct.tryNow()
  const peer = new Peer()
  attempts[0]!.resolve(peer)
  await flush()
  let closed = false
  void peer.closed.then(() => {
    closed = true
  })

  direct.reintroduce()
  await flush()
  expect(closed).toBe(true)
  expect(choices).toEqual(['direct', 'relay'])
  expect(attempts).toHaveLength(2)
  expect(waiting()).toBeUndefined()

  // An attempt under way was answered with the old introduction: a new one
  // takes its place, and the old one's peer is not used.
  direct.reintroduce()
  expect(attempts).toHaveLength(3)
  const stale = new Peer()
  attempts[1]!.resolve(stale)
  await flush()
  expect(direct.choice).toBe('relay')
  attempts[2]!.resolve(new Peer())
  await flush()
  expect(direct.choice).toBe('direct')
})

test('Server Only closes the peer and nothing tries again until the route allows one', async () => {
  const { direct, attempts, timers, choices } = device()
  direct.tryNow()
  const peer = new Peer()
  attempts[0]!.resolve(peer)
  await flush()
  expect(direct.choice).toBe('direct')

  direct.setRoute('server')
  let closed = false
  void peer.closed.then(() => {
    closed = true
  })
  await flush()
  expect(closed).toBe(true)
  expect(direct.choice).toBe('relay')
  expect(choices).toEqual(['direct', 'relay'])
  // No wait runs, and no reason to try makes a peer.
  expect(timers.filter((timer) => !timer.cancelled)).toEqual([])
  direct.tryNow()
  direct.setPermission('granted')
  expect(attempts).toHaveLength(1)

  direct.setRoute('automatic')
  expect(attempts).toHaveLength(2)
})

test('Server Only while an attempt runs leaves its peer unused', async () => {
  const { direct, attempts } = device()
  direct.tryNow()
  direct.setRoute('server')
  const peer = new Peer()
  let closed = false
  void peer.closed.then(() => {
    closed = true
  })
  attempts[0]!.resolve(peer)
  await flush()
  expect(direct.choice).toBe('relay')
  expect(closed).toBe(true)
})

test('under Automatic a slower direct path leaves its peer standing but unused; Prefer Direct uses it all the same', async () => {
  const { direct, attempts, choices } = device()
  direct.tryNow()
  const peer = new Peer()
  attempts[0]!.resolve(peer)
  await flush()
  expect(direct.current()).toBe(peer)

  direct.setSlower(true)
  expect(direct.choice).toBe('relay')
  expect(direct.current()).toBeNull()
  expect(direct.connected()).toBe(peer)
  direct.setRoute('direct')
  expect(direct.current()).toBe(peer)
  direct.setRoute('automatic')
  direct.setSlower(false)
  expect(direct.current()).toBe(peer)
  expect(choices).toEqual(['direct', 'relay', 'direct', 'relay', 'direct'])
})
