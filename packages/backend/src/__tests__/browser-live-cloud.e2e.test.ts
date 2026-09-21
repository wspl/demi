import { expect, test } from 'bun:test'
import { z } from 'zod'
import { nativeTargetSchema } from '@demicodes/command-protocol'
import { nativePackageFixture } from '@demicodes/host-remote/testing'
import { RemoteProvisioner } from '@demicodes/machines'
import { waitFor } from '@demicodes/utils'
import { World } from './scenarios/world'
import { model } from './scenarios/driver'
import { liveSite, LiveView } from './fixtures/live-view'

/**
 * The live browser view on a real Cloud guest (`browser-live-view.md` §
 * Acceptance): the same pages and viewer as the paired-device run, through a
 * gVisor sandbox's runner, including the guest's fonts for Chinese,
 * Japanese and Korean text. Run it with `DEMI_BROWSER_LIVE_CLOUD_E2E=1`.
 */
const acceptance = process.env.DEMI_BROWSER_LIVE_CLOUD_E2E === '1' ? test : test.skip

acceptance('a viewer watches a Cloud browser, types into it and ends it', async () => {
  const configuration = z.object({
    /** The guest's own `demi-commands`, which it seeds and the backend catalogs. */
    DEMI_NATIVE_TEST_BINARY: z.string().min(1),
    DEMI_BROWSER_CLOUD_SOCKET: z.string().min(1),
    DEMI_BROWSER_CLOUD_PUBLIC: z.url(),
    DEMI_BROWSER_CLOUD_TARGET: nativeTargetSchema,
  }).parse(process.env)
  // The catalog of the guest's own binary; the browser's release packaging is
  // the Cloud browser acceptance's subject, not this one's.
  const catalog = await nativePackageFixture()
  const artifact = catalog.packages[0]!.targets[configuration.DEMI_BROWSER_CLOUD_TARGET]
  if (!artifact)
    throw new Error(`The native fixture lacks ${configuration.DEMI_BROWSER_CLOUD_TARGET}`)
  const path = `/opt/demi/artifacts/${artifact.sha256}/demi-commands`
  const publicUrl = new URL(configuration.DEMI_BROWSER_CLOUD_PUBLIC)
  const site = liveSite({
    host: publicUrl.hostname,
    artifact: configuration.DEMI_NATIVE_TEST_BINARY,
    paintMs: 100,
  })
  const world = await World.create({
    port: Number(publicUrl.port), publicUrl: publicUrl.origin,
    nativeCommands: {
      packages: catalog.packages,
      resolveArtifact: async (requested, signal) => {
        signal.throwIfAborted()
        if (requested.sha256 !== artifact.sha256 || requested.size !== artifact.size) {
          throw new Error('Unexpected Cloud browser artifact')
        }
        return { path }
      },
    },
    managedHosts: {
      provisioner: new RemoteProvisioner({ socketPath: configuration.DEMI_BROWSER_CLOUD_SOCKET }),
      config: { sweepMs: 50 },
    },
  })
  const records = (type: string) => site.of(type)
  try {
    const driver = await world.conversation('cloud')
    // The guest carries the commands the resolver names, and the fonts the
    // view needs for Chinese, Japanese and Korean pages.
    const prepared = await driver.turn({
      model: [
        model.shell('prepare', `set -euxo pipefail
sudo -n mkdir -p $(dirname ${path})
sudo -n curl -fsS '${new URL('/artifact', site.url).href}' -o ${path}
sudo -n chmod 0755 ${path}
sha256sum ${path}
{ fc-list 2>/dev/null || ls /usr/share/fonts/opentype/noto 2>/dev/null; } | grep -i cjk | head -3 || true`, 600_000),
        model.say('prepared'),
      ],
    })
    expect(prepared.received[0]).toContain('exitCode: 0')
    expect(prepared.received[0]).toContain(artifact.sha256)
    expect(prepared.received[0]).toContain('CJK')
    console.info('Cloud guest carries the commands and the CJK fonts')

    const opened = await driver.turn({
      model: [
        model.shell('open', `demi browser open '${site.url}' --json`, 300_000),
        model.say('opened'),
      ],
    })
    expect(opened.received[0]).toContain('"tab"')
    await waitFor(() => records('ready').length > 0, () => 'the page never loaded', { timeoutMs: 120_000 })

    const view = new LiveView({ url: world.url, cookie: world.backend.session.cookie }, driver.id)
    await view.open()
    // A guest's two processors encode its pictures in software, so this view
    // watches at the ratio of an ordinary screen rather than a dense one.
    view.send({
      type: 'panel', width: 800, height: 600, devicePixelRatio: 1,
      screenWidth: 1440, screenHeight: 900,
    })
    const state = await view.message('state')
    expect(state.running).toBe(true)
    const tabs = state.tabs as Array<{ id: string; createdBy: { kind: string } }>
    expect(tabs).toHaveLength(1)
    const tab = tabs[0]!.id

    // Pictures of the guest's Chrome, at the viewer's panel size and ratio.
    view.send({ type: 'watch', tab })
    await waitFor(
      () => view.messages.some((message) => message.type === 'stream' && message.width === 800),
      () => `no stream at the panel size; saw ${view.messages.map((message) => message.type).join(', ')}`,
      { timeoutMs: 60_000 },
    )
    const stream = view.last('stream')!
    expect(stream.height).toBe(600)
    const generation = stream.generation as number
    await waitFor(() => view.frames.some((frame) => frame.generation === generation),
      () => 'no picture', { timeoutMs: 60_000 })
    const first = view.frames.find((frame) => frame.generation === generation)!
    expect(first.key).toBe(true)
    expect([first.width, first.height]).toEqual([800, 600])
    expect([...first.data.subarray(0, 4)]).toEqual([0, 0, 0, 1])
    const painted = view.frames.length
    await waitFor(() => view.frames.length > painted + 3, () => 'the moving page stopped', { timeoutMs: 30_000 })
    await waitFor(() => records('ready').at(-1)?.devicePixelRatio === 1,
      () => `ratios: ${records('ready').map((record) => record.devicePixelRatio).join(',')}`,
      { timeoutMs: 30_000 })
    expect(records('ready').at(-1)!.inner).toEqual([800, 600])
    console.info('Cloud pictures arrive at the viewer\'s panel size and ratio')

    // The viewer's input and the agent's commands reach the same page.
    view.pointer(tab, 'down', 100, 136)
    view.pointer(tab, 'up', 100, 136)
    view.key(tab, 'h', 'KeyH', 72, 'h')
    view.key(tab, 'i', 'KeyI', 73, 'i')
    await waitFor(() => records('input').some((record) => record.target === 'field' && record.value === 'hi'),
      () => `field records: ${JSON.stringify(records('input'))}`, { timeoutMs: 60_000 })
    // A guest with two processors encodes its pictures in the same Chrome that
    // serves the agent's commands, and those commands hold the Host's short
    // control deadlines however long the command itself may take. This view
    // therefore stops watching while the agent works; a viewer and an agent on
    // one tab at once is what the module's own tests and the paired-device run
    // cover.
    view.send({ type: 'watch', tab: null })
    const filled = await driver.turn({
      model: [
        model.shell('fill', `demi browser fill ${tab} --css '#area' --text agent --timeout 120000 --json`, 180_000),
        model.say('filled'),
      ],
    })
    expect(filled.received[0]).toContain('exitCode: 0')
    await waitFor(() => records('input').some((record) => record.target === 'area' && record.value === 'agent'),
      () => `area records: ${JSON.stringify(records('input').filter((record) => record.target === 'area'))}`,
      { timeoutMs: 60_000 })
    const resumed = view.messages.length
    view.send({ type: 'watch', tab })
    await waitFor(() => view.messages.slice(resumed).some((message) => message.type === 'stream'),
      () => 'the pictures never came back', { timeoutMs: 60_000 })
    view.pointer(tab, 'down', 100, 190)
    view.pointer(tab, 'up', 100, 190)
    view.key(tab, 'x', 'KeyX', 88, 'x')
    await waitFor(() => records('input').some((record) => record.target === 'area' && record.value === 'agentx'),
      () => `area records: ${JSON.stringify(records('input').filter((record) => record.target === 'area'))}`,
      { timeoutMs: 60_000 })
    console.info('Cloud input from the viewer and the agent reaches the page')

    // A page of Chinese, Japanese and Korean text draws its own glyphs.
    const fontsTab = await driver.turn({
      model: [
        model.shell('fonts', `demi browser open '${site.fontsUrl}' --json`, 300_000),
        model.say('fonts opened'),
      ],
    })
    expect(fontsTab.received[0]).toContain('"tab"')
    await waitFor(() => records('fonts').length > 0,
      () => `the fonts page never reported; the Host said ${JSON.stringify(view.messages.slice(-6))}`,
      { timeoutMs: 120_000 })
    const fonts = records('fonts').at(-1)!
    expect(fonts.samples).toHaveLength(3)
    for (const sample of fonts.samples!) {
      expect({ pair: sample.pair, drawn: sample.first.signature !== sample.second.signature })
        .toEqual({ pair: sample.pair, drawn: true })
      expect(Math.min(sample.first.ink, sample.second.ink)).toBeGreaterThan(40)
    }
    console.info('Cloud pages draw Chinese, Japanese and Korean text')

    // Ending the browser ends the view with it. The agent closes the tabs,
    // with the deadline a guest this size needs; a viewer's own close has the
    // Host's short control deadline, which the module's own tests cover.
    const opens = () => view.last('state')?.tabs as Array<{ id: string }> | undefined
    await waitFor(() => (opens()?.length ?? 0) === 2, () => `tabs: ${JSON.stringify(opens())}`,
      { timeoutMs: 60_000 })
    const closed = await driver.turn({
      model: [
        model.shell('close', opens()!
          .map((open) => `demi browser close ${open.id} --timeout 120000 --json`)
          .join('\n'), 300_000),
        model.say('closed'),
      ],
    })
    expect(closed.received[0]).toContain('exitCode: 0')
    const ended = await view.message('ended')
    expect(ended.reason).toBe('browser_ended')
    await waitFor(() => view.closed !== null, () => 'the stream stayed open', { timeoutMs: 60_000 })
    expect(view.closed).toMatchObject({ code: 1000 })
  } finally {
    site.stop()
    await world.close()
  }
}, 1_800_000)
