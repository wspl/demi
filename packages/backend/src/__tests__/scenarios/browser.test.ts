import { readFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { CONVERSATION_IDLE_MS } from '../../lifecycle/coordinator'
import { waitFor } from '@demicodes/utils'
import { model } from './driver'
import { World } from './world'

test.skipIf(process.env.DEMI_BROWSER_ACCEPTANCE !== '1')('AgentServer operates a retained browser through declared shell commands', async () => {
  const application = Bun.serve({
    port: 0,
    fetch(request) {
      const url = new URL(request.url)
      const html = url.pathname === '/dashboard'
        ? '<!doctype html><title>Dashboard</title><h1>Signed in</h1><p>Browser workflow completed</p><label>Enabled<input type="checkbox"></label><label>Country<select><option value="US">United States</option><option value="SG">Singapore</option></select></label><button onclick="document.querySelector(\'output\').textContent=prompt(\'Your name\',\'Guest\')">Prompt</button><output></output>'
        : '<!doctype html><title>Login</title><form action="/dashboard"><label>Email<input name="email"></label><label>Password<input name="password" type="password"></label><button>Sign in</button></form>'
      return new Response(html, { headers: { 'content-type': 'text/html; charset=utf-8' } })
    },
  })
  const world = await World.create({ runners: ['browser'] })
  world.selection.model.model.acceptedExtensions = ['png']
  try {
    const driver = await world.conversation('runner:browser')
    const opened = await driver.turn({ model: [
      model.shell('browser-open', `set -euo pipefail
tab=$(demi browser open http://127.0.0.1:${application.port}/ --json | jq -r .tab)
printf '%s' "$tab" > browser-tab.txt
printf 'Tab: %s\\n' "$tab"
demi browser inspect "$tab"
demi browser fill "$tab" --label Email --text agent@example.test
demi browser fill "$tab" --label Password --text fixture-password
demi browser click "$tab" --role button --name 'Sign in' --wait-url '**/dashboard*'
demi browser inspect "$tab"`, 120000),
      model.say('Page opened and form submitted'),
    ] })
    expect(opened.received[0]).toContain('exitCode: 0')
    expect(opened.received[0]).toContain('Signed in')
    expect(opened.received[0]).toMatch(/t_[A-Za-z0-9_-]{22}/)
    expect(opened.received[0]).toMatch(/e_[A-Za-z0-9_-]{22}/)
    await driver.detach()
    await driver.attach()
    const retained = await driver.turn({ model: [
      model.shell('browser-retained', `set -euo pipefail
tab=$(cat browser-tab.txt)
demi browser tabs --json
demi browser inspect "$tab"
demi browser screenshot "$tab" --output browser-shot.png --json`, 30000),
      model.say('Retained browser checked'),
    ] })
    expect(retained.received[0]).toContain('exitCode: 0')
    expect(retained.received[0]).toContain('Signed in')
    expect((await readFile(join(world.device('browser').home, 'browser-shot.png'))).subarray(0, 8))
      .toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))
    const screenshot = await driver.turn({ model: [
      model.shell('browser-image', 'demi browser screenshot "$(cat browser-tab.txt)"', 30000),
      model.say('Screenshot received by the model'),
    ] })
    expect(screenshot.received[0]).toContain('exitCode: 0')
    expect(screenshot.received[0]).toContain('[image]')
    const controls = await driver.turn({ model: [
      model.shell('browser-controls', `set -euo pipefail
tab=$(cat browser-tab.txt)
demi browser check "$tab" --label Enabled --value
demi browser select "$tab" --label Country --option-label Singapore
demi browser read "$tab" --label Country --property value --json
demi browser click "$tab" --role heading --name 'Signed in' --wait-url '**/dashboard*' --json
demi browser click "$tab" --role heading --name 'Signed in' --wait-url '**/never' --timeout 700 --json || true
demi browser info t_AAAAAAAAAAAAAAAAAAAAAA --json || true
demi browser scroll "$tab" --dy 100
demi browser content read "$tab" --format html --output page.html --json
demi browser click "$tab" --role button --name Prompt --json || true
demi browser dialog inspect "$tab" --json
demi browser dialog accept "$tab" --text Agent --json
demi browser read "$tab" --css output --property text --json`, 30000),
      model.say('Additional page controls checked'),
    ] })
    expect(controls.received[0]).toContain('exitCode: 0')
    expect(controls.received[0]).toContain('"value":"SG"')
    expect(controls.received[0]).toContain('"code":"timeout"')
    expect(controls.received[0]).toContain('dialog_blocked')
    expect(controls.received[0]).toContain('"action":"completed"')
    expect(controls.received[0]).toContain('"code":"tab_not_found"')
    expect(controls.received[0]).toContain('Your name')
    expect(controls.received[0]).toContain('"value":"Agent"')
    const closed = await driver.turn({ model: [
      model.shell('browser-close', 'demi browser close "$(cat browser-tab.txt)" --json', 30000),
      model.say('Browser closed'),
    ] })
    expect(closed.received[0]).toContain('exitCode: 0')
    const empty = await driver.turn({ model: [
      model.shell('browser-empty', 'demi browser tabs --json', 30000),
      model.say('Fresh controller has no tabs'),
    ] })
    expect(empty.received[0]).toContain('"tabs":[]')
    expect(world.frames.some(frame => frame.message.type === 'job_start' && frame.message.context.conversation === driver.id && frame.message.context.caller.kind === 'agent' && frame.message.context.caller.node === driver.id)).toBe(true)
  } finally {
    await world.close()
    application.stop(true)
  }
}, 180000)

test.skipIf(process.env.DEMI_BROWSER_ACCEPTANCE !== '1')('a retained paired browser is retired after one hour of conversation inactivity', async () => {
  const application = Bun.serve({ port: 0, fetch: () => new Response('<h1>Idle browser</h1>', { headers: { 'content-type': 'text/html' } }) })
  let now = 0
  const world = await World.create({ runners: ['browser-idle'], lifecycle: { now: () => now, pollMs: 10 } })
  try {
    const driver = await world.conversation('runner:browser-idle')
    const opened = await driver.turn({ model: [
      model.shell('open-idle', `demi browser open http://127.0.0.1:${application.port}/ --timeout 120000 --json`, 120000),
      model.say('Browser is ready to become idle'),
    ] })
    expect(opened.received[0]).toContain('exitCode: 0')
    // The process tree identifies this fixture's Chrome without a browser-specific backend API.
    const processSnapshot = async () => {
      const child = Bun.spawn(['ps', '-axo', 'pid=,ppid=,command='], { stdout: 'pipe' })
      const output = await new Response(child.stdout).text()
      expect(await child.exited).toBe(0)
      return output.split('\n').flatMap(line => {
        const match = /^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line)
        return match ? [{ pid: Number(match[1]), parent: Number(match[2]), command: match[3]! }] : []
      })
    }
    const snapshot = await processSnapshot()
    const service = snapshot.find(row => row.command.includes(world.device('browser-idle').stateDir) && row.command.includes('--command-service'))
    expect(service).toBeDefined()
    const chrome = snapshot.find(row => row.parent === service?.pid && row.command.includes('--user-data-dir='))
    const profile = /--user-data-dir=([^\s]+)/.exec(chrome?.command ?? '')?.[1]
    if (!profile) throw new Error('Fixture Chrome profile is missing')
    expect(profile).toContain('demi-browser-')
    expect(existsSync(profile)).toBe(true)
    await Bun.sleep(30)
    now = CONVERSATION_IDLE_MS - 1
    await Bun.sleep(30)
    expect(world.frames.some(frame => frame.message.type === 'conversation_release')).toBe(false)
    now++
    await waitFor(() => world.frames.some(frame => frame.message.type === 'conversation_released' && !frame.message.error))
    expect(existsSync(profile)).toBe(false)
    expect((await processSnapshot()).some(row => row.command.includes(`--user-data-dir=${profile}`))).toBe(false)
    const next = await driver.turn({ model: [
      model.shell('idle-tabs', 'demi browser tabs --json', 30000),
      model.say('Browser was reclaimed without stopping the paired runner'),
    ] })
    expect(next.received[0]).toContain('"tabs":[]')
  } catch (error) {
    console.error('Idle continuation failed', error)
    throw error
  } finally {
    await world.close()
    application.stop(true)
  }
}, 180_000)

test.skipIf(process.env.DEMI_BROWSER_ACCEPTANCE !== '1')('switch and archive release browser environments on main and attached Hosts', async () => {
  const world = await World.create({ runners: ['one', 'two'] })
  try {
    const driver = await world.conversation('runner:one')
    const opened = await driver.turn({ model: [
      model.shell('first-browser', 'demi browser open about:blank --json', 120_000),
      model.say('opened'),
    ] })
    expect(opened.received[0]).toContain('exitCode: 0')
    await driver.switchTo('runner:two')
    const afterSwitch = await driver.turn({ model: [
      model.shell('two-browsers', `set -e
demi host shell --host one 'demi browser tabs --json; demi browser open about:blank --json'
demi browser open about:blank --json`, 120_000),
      model.say('independent environments'),
    ] })
    expect(afterSwitch.received[0]).toContain('exitCode: 0')
    expect(afterSwitch.received[0]).toContain('"tabs":[]')
    await world.api(`/api/conversations/${driver.id}`, { archived: true }, 'PATCH')
    await world.api(`/api/conversations/${driver.id}`, { archived: false }, 'PATCH')
    const afterArchive = await driver.turn({ model: [
      model.shell('released-browsers', `demi browser tabs --json; demi host shell --host one 'demi browser tabs --json'`, 30_000),
      model.say('both released'),
    ] })
    expect(afterArchive.received[0]).toContain('exitCode: 0')
    expect(afterArchive.received[0]?.match(/"tabs":\[\]/g)).toHaveLength(2)
  } finally {
    await world.close()
  }
}, 180_000)
