import { rm } from 'node:fs/promises'
import { expect, test } from 'bun:test'
import { z } from 'zod'
import { delay, waitFor } from '@demicodes/utils'
import { cloudImageManifestSchema, RemoteProvisioner } from '@demicodes/machines'
import { World } from './scenarios/world'
import { model } from './scenarios/driver'
import { LiveView } from './fixtures/live-view'

// Uses a separately installed Linux manager, real release artifacts, and a scripted model.
const e2e = process.env.DEMI_GVISOR_E2E === '1' ? test : test.skip
const cloudStatus = z.object({ device: z.object({ id: z.string() }), operation: z.object({ phase: z.string() }).nullable() })

/** Connect a test backend to the Cloud manager and its embedded command catalog. */
async function managedWorld() {
  const config = z.object({
    DEMI_GVISOR_E2E_SOCKET: z.string().min(1),
    DEMI_GVISOR_E2E_PUBLIC: z.url(),
    DEMI_GVISOR_E2E_MANIFEST: z.string().min(1),
  }).parse(process.env)
  const image = cloudImageManifestSchema.parse(await Bun.file(config.DEMI_GVISOR_E2E_MANIFEST).json())
  const provisioner = new RemoteProvisioner({ socketPath: config.DEMI_GVISOR_E2E_SOCKET })
  const world = await World.create({
    port: Number(new URL(config.DEMI_GVISOR_E2E_PUBLIC).port),
    publicUrl: config.DEMI_GVISOR_E2E_PUBLIC,
    lifecycle: { idleMs: 600_000 },
    nativeCommands: {
      packages: image.releases,
      resolveArtifact: async (artifact, signal) => {
        signal.throwIfAborted()
        const path = Object.entries(image.executables).find(([path, entry]) =>
          path.startsWith(`/opt/demi/artifacts/${artifact.sha256}/`) && entry.sha256 === artifact.sha256 && entry.size === artifact.size)?.[0]
        if (!path) throw new Error('Artifact is outside the Cloud image catalog')
        return { path }
      },
    },
    managedHosts: { provisioner, config: { sweepMs: 600_000 } },
  })
  return { world, provisioner, publicUrl: new URL(config.DEMI_GVISOR_E2E_PUBLIC) }
}

e2e('Cloud browser, volume growth, paired checkpoint, hibernate and external system reset', async () => {
  const { world, provisioner, publicUrl } = await managedWorld()
  const forbidden = Bun.serve({ port: 0, hostname: '0.0.0.0', fetch: () => new Response('private service') })
  let view: LiveView | undefined
  try {
    const firstCommandAt = performance.now()
    const driver = await world.conversation('cloud')
    const uploaded = await driver.upload('uploaded', new TextEncoder().encode('from-api'))
    const uploadedPath = driver.attachmentPath('uploaded')
    const initial = await driver.turn({
      content: [{ type: 'text', text: 'go' }, uploaded],
      model: [model.shell('initial', `set -e
id -u
stat -c %u ${uploadedPath}
echo home-data > note
sudo sh -c "echo installed > /usr/local/system-marker"
demi file read ${uploadedPath}
uv --version
cat > browser.html <<'HTML'
<!doctype html><meta charset="utf-8"><input id="field" style="position:absolute;left:20px;top:120px;width:320px;height:32px"><p>中文 日本語 한국어</p>
HTML
printf 'file://%s/browser.html\n' "$PWD"`), model.say('ready')],
    })
    expect(initial.received[0]).toContain('1000\n1000')
    expect(initial.received[0]).toContain('from-api')
    console.info('Cloud first command (ms)', Math.round(performance.now() - firstCommandAt))
    const page = initial.received[0]!.match(/file:\/\/[^\n]+\/browser.html/)?.[0]
    expect(page).toBeDefined()
    const status = cloudStatus.parse(await world.api('/api/cloud'))
    const second = z.object({ workspace: z.object({ deviceId: z.string() }) }).parse(
      await world.api('/api/workspaces', { cloud: true, name: 'other-project' }))
    expect(second.workspace.deviceId).toBe(status.device.id)
    const network = await driver.turn({ model: [model.shell('network', `set -e
curl -fsS --max-time 30 https://example.com >/dev/null
python3 - <<'PY'
import ipaddress
import socket
targets = [('169.254.169.254', 80), ('10.0.0.1', 80)]
# A live private service on the backend host proves the endpoint allowlist.
backend = socket.gethostbyname(${JSON.stringify(publicUrl.hostname)})
if not ipaddress.ip_address(backend).is_global:
    targets.append((backend, ${forbidden.port}))
for target in targets:
    try:
        connection = socket.create_connection(target, timeout=1)
    except OSError:
        continue
    connection.close()
    raise RuntimeError('Cloud reached forbidden destination: ' + repr(target))
print('public-egress-and-private-denial-ok')
PY`), model.say('network checked')] })
    expect(network.received[0]).toContain('public-egress-and-private-denial-ok')

    const browserAt = performance.now()
    const opened = z.object({ id: z.string() }).parse(await world.api(`/api/conversations/${driver.id}/browser/tabs`, { url: page }))
    console.info('Cloud browser open (ms)', Math.round(performance.now() - browserAt))
    view = new LiveView({ url: world.url, cookie: world.backend.session.cookie }, driver.id)
    const frameAt = performance.now()
    await view.open()
    view.send({ type: 'panel', width: 800, height: 600, devicePixelRatio: 1, screenWidth: 1440, screenHeight: 900 })
    await view.message('state')
    view.send({ type: 'watch', tab: opened.id })
    await waitFor(() => view!.frames.length > 0, () => JSON.stringify(view!.messages), { timeoutMs: 30_000 })
    console.info('Cloud first live frame (ms)', Math.round(performance.now() - frameAt))
    expect([view.frames[0]!.width, view.frames[0]!.height]).toEqual([800, 600])
    view.pointer(opened.id, 'down', 100, 136)
    view.pointer(opened.id, 'up', 100, 136)
    view.key(opened.id, 'h', 'KeyH', 72, 'h')
    view.key(opened.id, 'i', 'KeyI', 73, 'i')
    const typed = await driver.turn({ model: [
      model.shell('read-input', `demi browser eval ${opened.id} <<'JS'
document.querySelector('#field').value
JS`), model.say('read'),
    ] })
    expect(typed.received[0]).toContain('hi')
    await provisioner.growVolume(status.device.id, 'system', 2 * 1024 ** 3)
    await provisioner.growVolume(status.device.id, 'home', 2 * 1024 ** 3)
    const saveAt = performance.now()
    await provisioner.checkpoint(status.device.id)
    console.info('Cloud checkpoint (ms)', Math.round(performance.now() - saveAt))
    const saved = await provisioner.imageState(status.device.id)
    expect(saved?.systemBytes).toBe(2 * 1024 ** 3)
    expect(saved?.homeBytes).toBe(2 * 1024 ** 3)
    view.close()
    view = undefined
    await world.backend.managedHosts!.hibernate(status.device.id)
    const woken = await driver.turn({ model: [model.shell('wake',
      'cat note /usr/local/system-marker; echo latest-home > note; sudo chmod 000 /usr/bin/bash'), model.say('awake')] })
    expect(woken.received[0]).toContain('home-data\ninstalled')
    await world.api('/api/cloud/reset', { operationId: crypto.randomUUID() })
    const deadline = Date.now() + 60_000
    while (cloudStatus.parse(await world.api('/api/cloud')).operation?.phase !== 'ready') {
      if (Date.now() > deadline) throw new Error('Cloud reset did not finish')
      await delay(50)
    }
    const reset = await driver.turn({ model: [model.shell('reset',
      'cat note; test ! -e /usr/local/system-marker && echo clean-system'), model.say('reset')] })
    expect(reset.received[0]).toContain('latest-home')
    expect(reset.received[0]).toContain('clean-system')
    expect(cloudStatus.parse(await world.api('/api/cloud')).device.id).toBe(status.device.id)
  } finally {
    view?.close()
    await forbidden.stop(true)
    await world.close()
    await rm(world.dataDir, { recursive: true, force: true })
  }
}, 240_000)

// The login shell on the real guest (`runner.md` § Jobs and the tee): a
// toolchain installed by one command is on PATH for the next, with nothing
// sourced — rustup through `~/.profile`, nvm through the skeleton's
// `~/.bashrc`. Needs egress from the guest; several minutes.
e2e(
  'managed runner: rustup in one command and cargo in the next; nvm likewise',
  async () => {
    const { world } = await managedWorld()
    try {
      const driver = await world.conversation('cloud')
      const facts = await driver.turn({ model: [
          model.shell('facts', 'ls -A ~; uv --version; node --version'),
          model.say('facts')
        ] })
      expect(facts.received[0]).toContain('.profile')
      expect(facts.received[0]).toContain('uv 0.')
      const rustup = await driver.turn({ model: [
          model.shell(
            'rustup',
            "set -o pipefail; curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal > /dev/null 2>&1; echo rustup-exit=$?",
            600_000
          ),
          model.say('installed')
        ] })
      expect(rustup.received[0]).toContain('rustup-exit=0')
      const cargo = await driver.turn({ model: [
          model.shell('cargo', 'cargo --version'),
          model.say('cargo')
        ] })
      expect(cargo.received[0]).toContain('cargo 1.')
      const nvm = await driver.turn({ model: [
          model.shell(
            'nvm',
            'curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.7/install.sh | bash > /dev/null 2>&1; . ~/.bashrc; nvm install --lts > /dev/null 2>&1; echo nvm-exit=$?',
            600_000
          ),
          model.say('nvm')
        ] })
      expect(nvm.received[0]).toContain('nvm-exit=0')
      const node = await driver.turn({ model: [
          model.shell('node', 'node --version; command -v node'),
          model.say('node')
        ] })
      expect(node.received[0]).toContain('/.nvm/versions/node/')
    } finally {
      await world.close()
      await rm(world.dataDir, { recursive: true, force: true })
    }
  },
  1_200_000
)
