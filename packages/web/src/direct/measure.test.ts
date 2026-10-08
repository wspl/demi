import { expect, test } from 'bun:test'
import { AutomaticChoice, PathProbes, directWorse } from './measure'

// Automatic's choice from the probes of both paths, once a second, on a
// clock the test plays (`direct-channel.md` § Measuring the paths). Pure;
// milliseconds.

/** Both paths probed each second, each answering as `rtt` says for that second; null loses it. */
function paths() {
  const direct = new PathProbes(true)
  const relay = new PathProbes(false)
  const choice = new AutomaticChoice()
  let now = 0
  const slower: boolean[] = []
  /** Runs `seconds` seconds of probes and records Automatic's choice each second. */
  const run = (seconds: number, directRtt: (second: number) => number | null, relayRtt = 30) => {
    for (let second = 0; second < seconds; second++) {
      const sentDirect = direct.send(now)
      const sentRelay = relay.send(now)
      const rtt = directRtt(second)
      if (rtt !== null)
        direct.answered(sentDirect, now + rtt)
      relay.answered(sentRelay, now + relayRtt)
      now += 1000
      direct.expire(now + 1000)
      relay.expire(now + 1000)
      slower.push(choice.update(now, directWorse(direct.recent(now), relay.recent(now))))
    }
  }
  return { direct, relay, run, slower }
}

test('one lost probe does not move the choice off the direct path', () => {
  const { run, slower } = paths()
  run(20, () => 2)
  // A tenth of the next ten seconds' probes is lost, more than 2%, but for
  // less than ten seconds.
  run(1, () => null)
  run(20, () => 2)
  expect(slower.every((value) => !value)).toBe(true)
})

test('a direct path slower for ten seconds gives way to the relay, and takes over again once it has recovered for ten', () => {
  const { run, slower } = paths()
  run(10, () => 2)
  run(30, () => 80)
  // Worse within the first window, then held for ten seconds before moving.
  expect(slower.slice(10, 15).some((value) => value)).toBe(false)
  expect(slower.at(-1)).toBe(true)
  const movedAt = slower.indexOf(true)
  expect(movedAt).toBeGreaterThanOrEqual(19)

  run(30, () => 2)
  expect(slower.at(-1)).toBe(false)
  expect(slower.slice(40, 49).every((value) => value)).toBe(true)
})

test('a direct path losing more than 2% of its probes for ten seconds gives way to the relay', () => {
  const { run, slower, direct } = paths()
  run(10, () => 2)
  // One probe in five lost: 20% of the last ten seconds.
  run(30, (second) => (second % 5 === 0 ? null : 2))
  expect(direct.figures()?.loss).toBeGreaterThan(0.02)
  expect(slower.at(-1)).toBe(true)
})

test('the figures are the median round trip and the share lost, the relay losing none', () => {
  const { run, direct, relay } = paths()
  run(4, (second) => [2, 4, null, 10][second]!)
  expect(direct.figures()).toEqual({ latencyMs: 4, loss: 0.25 })
  expect(relay.figures()).toEqual({ latencyMs: 30, loss: null })
})
