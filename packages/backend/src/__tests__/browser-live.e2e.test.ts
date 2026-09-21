import { afterAll, beforeAll, expect, test } from 'bun:test'
import { nativePackageFixture } from '@demicodes/host-remote/testing'
import { waitFor } from '@demicodes/utils'
import { World } from './scenarios/world'
import { model } from './scenarios/driver'
import { liveSite, LiveView, type LiveRecord, type LiveSite } from './fixtures/live-view'

/**
 * The live browser view end to end (`browser-live-view.md` § Acceptance): a
 * page's stream through the backend and the runner to the conversation's own
 * Chrome, against fixture pages that record what they received. Run it with
 * `DEMI_BROWSER_LIVE_E2E=1`; it downloads the pinned Chrome on first use.
 */
const acceptance = process.env.DEMI_BROWSER_LIVE_E2E === '1' ? test : test.skip

let world: World
let site: LiveSite

beforeAll(async () => {
  const builtin = await nativePackageFixture()
  site = liveSite()
  world = await World.create({ runners: ['alpha'], nativeCommands: builtin })
}, 180_000)

afterAll(async () => {
  site?.stop()
  await world?.close()
}, 60_000)

/** What the fixture pages recorded, newest last. */
function records(type: string): LiveRecord[] {
  return site.of(type)
}

acceptance('a viewer watches the conversation\'s browser, types into it and ends it', async () => {
  const driver = await world.conversation('runner:alpha')
  const opened = await driver.turn({
    model: [
      model.shell('open', `demi browser open '${site.url}' --json`, 120_000),
      model.say('opened'),
    ],
  })
  expect(opened.received[0]).toContain('"tab"')
  await waitFor(() => records('ready').length > 0, () => 'the page never loaded', { timeoutMs: 60_000 })

  const view = new LiveView({ url: world.url, cookie: world.backend.session.cookie }, driver.id)
  await view.open()
  view.send({
    type: 'panel', width: 800, height: 600, devicePixelRatio: 2,
    screenWidth: 1440, screenHeight: 900,
  })
  const state = await view.message('state')
  expect(state.running).toBe(true)
  const tabs = state.tabs as Array<{ id: string; url: string; createdBy: { kind: string } }>
  expect(tabs).toHaveLength(1)
  expect(tabs[0]!.createdBy.kind).toBe('agent')
  const tab = tabs[0]!.id

  // 1. Pictures: watching a tab starts a stream at the panel's size, with a
  // key frame first, and a moving page keeps sending.
  view.send({ type: 'watch', tab })
  await waitFor(() => view.messages.some((message) => message.type === 'stream' && message.width === 1600),
    () => 'no stream at the panel size', { timeoutMs: 30_000 })
  const stream = view.last('stream')!
  expect(stream.height).toBe(1200)
  const generation = stream.generation as number
  await waitFor(() => view.frames.some((frame) => frame.generation === generation),
    () => 'no picture', { timeoutMs: 30_000 })
  const first = view.frames.find((frame) => frame.generation === generation)!
  expect(first.key).toBe(true)
  expect([first.width, first.height]).toEqual([1600, 1200])
  expect([...first.data.subarray(0, 4)]).toEqual([0, 0, 0, 1])
  const painted = view.frames.length
  await waitFor(() => view.frames.length > painted + 3, () => 'the moving page stopped', { timeoutMs: 20_000 })

  // 5. The page sees an ordinary browser at the viewer's ratio.
  await waitFor(() => records('ready').at(-1)?.devicePixelRatio === 2,
    () => `ratios: ${records('ready').map((record) => record.devicePixelRatio).join(',')}`,
    { timeoutMs: 20_000 })
  const ready = records('ready').at(-1)!
  expect(ready.inner).toEqual([800, 600])
  expect(ready).toMatchObject({ webdriver: false, headless: false, hover: true, finePointer: true })
  expect((ready as unknown as { scrollbar: number }).scrollbar).toBeGreaterThan(0)

  // 3. Input reaches the page as a local browser delivers it.
  const pointer = (action: string, x: number, y: number) => view.pointer(tab, action, x, y)
  const key = (value: string, code: string, keyCode: number, text?: string) =>
    view.key(tab, value, code, keyCode, text)
  pointer('down', 100, 136)
  pointer('up', 100, 136)
  key('h', 'KeyH', 72, 'h')
  key('i', 'KeyI', 73, 'i')
  key('Enter', 'Enter', 13)
  await waitFor(() => records('input').some((record) => record.target === 'field' && record.value === 'hi'),
    () => `field records: ${JSON.stringify(records('input'))}`, { timeoutMs: 20_000 })
  expect(records('keypress').some((record) => record.key === 'h')).toBe(true)
  expect(records('mousedown').some((record) => record.target === 'field')).toBe(true)

  // A wheel turn scrolls the CSS distance the viewer turned.
  view.send({ type: 'wheel', tab, x: 400, y: 300, deltaX: 0, deltaY: 200, modifiers: 0 })
  await waitFor(() => site.records.some((record) => (record.scrollY ?? 0) >= 200),
    () => `the page never scrolled; records: ${JSON.stringify(site.records.slice(-4))}`,
    { timeoutMs: 20_000 })

  // 4. The agent's commands and the viewer's input both take effect.
  const filled = await driver.turn({
    model: [
      model.shell('fill', `demi browser fill ${tab} --css '#area' --text agent --json`, 60_000),
      model.say('filled'),
    ],
  })
  expect(filled.received[0]).toContain('exitCode: 0')
  pointer('down', 100, 190)
  pointer('up', 100, 190)
  key('x', 'KeyX', 88, 'x')
  await waitFor(() => records('input').some((record) => record.target === 'area' && record.value === 'agentx'),
    () => `area records: ${JSON.stringify(records('input').filter((record) => record.target === 'area'))}`,
    { timeoutMs: 20_000 })

  // 8. A page that stops showing frames does not pile them up: the Host
  // stops sending past its window, and the page resumes from a key frame.
  view.acknowledge = false
  const held = view.frames.length
  await new Promise((resolve) => setTimeout(resolve, 3000))
  const whileHeld = view.frames.length - held
  expect(whileHeld).toBeLessThan(16)
  view.acknowledge = true
  view.send({ type: 'keyframe', generation })
  await waitFor(() => view.frames.slice(held + whileHeld).some((frame) => frame.key),
    () => 'no key frame after the page caught up', { timeoutMs: 20_000 })

  // 7. A Host that pauses stops the stream where it is: nothing arrives, not
  // even a heartbeat, and the view resumes from a key frame when it returns.
  const before = { frames: view.frames.length, beats: view.heartbeats }
  process.kill(world.device('alpha').runner.pid, 'SIGSTOP')
  await new Promise((resolve) => setTimeout(resolve, 2000))
  const during = {
    frames: view.frames.length - before.frames,
    beats: view.heartbeats - before.beats,
  }
  process.kill(world.device('alpha').runner.pid, 'SIGCONT')
  // Two seconds of a moving page are twenty-odd frames; a paused Host sends
  // only what was already in flight, which is the silence the view shows.
  expect(during.frames + during.beats).toBeLessThan(5)
  view.send({ type: 'keyframe', generation })
  await waitFor(() => view.frames.slice(before.frames + during.frames).some((frame) => frame.key),
    () => 'no key frame after the Host returned', { timeoutMs: 30_000 })
  await waitFor(() => view.frames.length > before.frames + during.frames + 8,
    () => 'the pictures never resumed', { timeoutMs: 30_000 })
  expect(view.closed).toBe(null)

  // 9. The panel's tab requests (`web-api.md` § Conversation browser tabs) run the agent's own
  // operations as the user: they list the agent's tab, open one without waiting for its page,
  // navigate it, and tell a tab the browser does not have.
  const tabRoutes = `/api/conversations/${driver.id}/browser/tabs`
  const json = (body: unknown) => ({ method: 'POST', body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })
  const listed = await (await world.backend.session.fetch(tabRoutes)).json() as { tabs: Array<{ id: string; createdBy: { kind: string } }> }
  expect(listed.tabs.map((item) => item.id)).toEqual([tab])
  const userTab = await world.backend.session.fetch(tabRoutes, json({}))
  expect(userTab.status).toBe(200)
  const mine = await userTab.json() as { id: string; url: string; createdBy: { kind: string } }
  expect(mine).toMatchObject({ url: 'about:blank', createdBy: { kind: 'user' } })
  expect((await world.backend.session.fetch(`${tabRoutes}/${mine.id}/navigate`, json({ url: site.url }))).status).toBe(204)
  expect((await world.backend.session.fetch(`${tabRoutes}/${mine.id}/history`, json({ action: 'reload' }))).status).toBe(204)
  const missing = await world.backend.session.fetch(`${tabRoutes}/t_nosuchtabnosuchtabnosu/navigate`, json({ url: site.url }))
  expect(missing.status).toBe(404)
  expect(await missing.json()).toMatchObject({ code: 'tab_not_found' })
  expect((await world.backend.session.fetch(`${tabRoutes}/${mine.id}`, { method: 'DELETE' })).status).toBe(204)
  // A tab the browser no longer has is closed.
  expect((await world.backend.session.fetch(`${tabRoutes}/${mine.id}`, { method: 'DELETE' })).status).toBe(204)

  // 7. Closing the last tab ends the browser, and the view says so.
  expect((await world.backend.session.fetch(`${tabRoutes}/${tab}`, { method: 'DELETE' })).status).toBe(204)
  const ended = await view.message('ended')
  expect(ended.reason).toBe('browser_ended')
  await waitFor(() => view.closed !== null, () => 'the stream stayed open', { timeoutMs: 30_000 })
  expect(view.closed).toMatchObject({ code: 1000 })
}, 300_000)
