// A call whose browser stops answering, with the slot's real browser frozen
// by SIGSTOP in a temporary slot: about 2 s, the browser's start and the
// second the call waits for an answer. No cheaper test shows that the call
// ends whatever its script waits on, and that the browser goes with it.
import { afterEach, expect, test } from 'bun:test'
import { existsSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'
import { Browser } from './browser'
import { runCall } from './call'
import { Network } from './network'
import { ownGroupRuns, stopGroup, type Group } from './processes'
import { slotPorts, type Slot } from './slot'
import { readState } from './state'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) {
    await cleanup()
  }
})

test.skipIf(!existsSync(chromium.executablePath()))(
  'a browser that stops answering ends the call within seconds, and the tool closes it',
  async () => {
    const root = mkdtempSync(join(tmpdir(), 'browse-hang-'))
    const slot: Slot = { root, number: 0, ports: slotPorts(0), folder: join(root, '.cache/browse') }
    mkdirSync(slot.folder, { recursive: true })
    const browser = new Browser(slot, () => undefined)
    const network = Network.listen(0, 1)
    let frozen: Group | undefined
    cleanups.push(async () => {
      if (frozen && ownGroupRuns(frozen)) {
        process.kill(-frozen.pgid, 'SIGCONT')
        await stopGroup(frozen, 0)
      }
      await (await network).shutdown()
      rmSync(root, { recursive: true, force: true })
    })
    const keep = {
      freeze: () => {
        frozen = readState(slot).browser
        process.kill(-frozen!.pgid, 'SIGSTOP')
      },
    }
    const lines: string[] = []
    // A key press waits for the browser with no timeout of Playwright's.
    const script = 'keep.freeze()\nawait page.keyboard.press("Escape")'
    const started = Date.now()
    const outcome = await runCall(
      { slot, browser, network: () => network, keep },
      { script, file: null, headed: false, env: {}, limitMs: 15_000 },
      (line) => lines.push(line),
      1_000,
    )
    expect(Date.now() - started).toBeLessThan(10_000)
    expect(lines).toEqual([
      'Failed: the browser did not answer for 1 s, so the call stopped waiting for the script',
      'Page      the browser did not answer, so the tool closed it; the next call starts a new one',
    ])
    expect(outcome).toEqual({ exit: 1, after: 'end' })
    expect([ownGroupRuns(frozen!), readState(slot).browser]).toEqual([false, undefined])
  },
)
