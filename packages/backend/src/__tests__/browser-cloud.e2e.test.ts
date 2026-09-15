import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { z } from 'zod'
import { nativePackageSchema, nativeTargetSchema } from '@demicodes/command-protocol'
import { RemoteProvisioner } from '@demicodes/machines'
import { delay } from '@demicodes/utils'
import { World } from './scenarios/world'
import { model } from './scenarios/driver'
import type { ManagedOperation } from '../storage/control'

const applicationUrl = new URL('http://127.0.0.1:8123/')
const acceptance = process.env.DEMI_BROWSER_CLOUD_E2E === '1' ? test : test.skip

acceptance('Agent browser runs in real Cloud, retires before reset/idle shutdown, and wakes with preserved files', async () => {
  const configuration = z.object({
    DEMI_BROWSER_PACKAGE: z.string().min(1),
    DEMI_BROWSER_CLOUD_SOCKET: z.string().min(1),
    DEMI_BROWSER_CLOUD_PUBLIC: z.url(),
    DEMI_BROWSER_CLOUD_TARGET: nativeTargetSchema,
  }).parse(process.env)
  const descriptor = nativePackageSchema.parse(await Bun.file(join(configuration.DEMI_BROWSER_PACKAGE, 'descriptor.json')).json())
  const artifact = descriptor.targets[configuration.DEMI_BROWSER_CLOUD_TARGET]
  const publicUrl = new URL(configuration.DEMI_BROWSER_CLOUD_PUBLIC)
  const world = await World.create({
    port: Number(publicUrl.port), publicUrl: publicUrl.origin,
    nativeCommands: {
      packages: [descriptor],
      resolveArtifact: async (requested, signal) => {
        signal.throwIfAborted()
        if (requested.sha256 !== artifact.sha256 || requested.size !== artifact.size)
          throw new Error('Unexpected Cloud browser artifact')
        return { path: `/opt/demi/artifacts/${artifact.sha256}/demi-commands` }
      },
    },
    managedHosts: {
      provisioner: new RemoteProvisioner({ socketPath: configuration.DEMI_BROWSER_CLOUD_SOCKET }),
      config: { idleMs: 1000, sweepMs: 50 },
    },
  })
  try {
    const driver = await world.conversation('cloud')
    const first = await driver.turn({ model: [
      model.shell('cloud-browser', withCloudApplication(`set -euxo pipefail
tab=$(demi browser open '${applicationUrl.href}' --json | jq -r .tab)
printf '%s' "$tab" > browser-tab.txt
printf retained > browser-marker.txt
demi browser inspect "$tab"
demi browser fill "$tab" --label Email --text cloud@example.test
demi browser click "$tab" --role button --name Continue --wait-url '**/complete/*'
demi browser inspect "$tab"
demi browser screenshot "$tab" --output cloud-browser.png --json
test -s cloud-browser.png`), 600000),
      model.say('Cloud browser operated'),
    ] })
    expect(first.received[0]).toContain('exitCode: 0')
    expect(first.received[0]).toContain('Cloud signed in')
    console.info('Cloud browser login and screenshot passed')
    const reset = await world.api<{ operation: ManagedOperation }>('/api/cloud/reset', { operationId: crypto.randomUUID() })
    const resetDeadline = performance.now() + 90_000
    while (true) {
      const state = await world.api<{ operation: ManagedOperation }>('/api/cloud')
      if (state.operation.phase === 'failed') throw new Error(state.operation.error ?? 'Cloud reset failed')
      if (state.operation.phase === 'ready') break
      if (performance.now() > resetDeadline) throw new Error('Cloud reset did not complete')
      await delay(100)
    }
    expect(reset.operation.id).toBeTruthy()
    expect(world.frames.some(frame => frame.message.type === 'resource_release')).toBe(true)
    const afterReset = await driver.turn({ model: [
      model.shell('cloud-browser-reset', withCloudApplication(`set -euxo pipefail
cat browser-marker.txt
demi browser tabs --json
demi browser open '${applicationUrl.href}' --json`), 360000),
      model.say('Cloud browser restarted after reset'),
    ] })
    expect(afterReset.received[0]).toContain('exitCode: 0')
    expect(afterReset.received[0]).toContain('retained')
    expect(afterReset.received[0]).toContain('"tabs":[]')
    console.info('Cloud reset preserved files and retired browser tabs')
    const idleDeadline = performance.now() + 60_000
    while ((await world.api<{ state: string }>('/api/cloud')).state !== 'off') {
      if (performance.now() > idleDeadline) throw new Error('Cloud idle shutdown did not complete')
      await delay(100)
    }
    expect(world.frames.filter(frame => frame.message.type === 'resource_release').length).toBeGreaterThanOrEqual(2)
    const woken = await driver.turn({ model: [
      model.shell('cloud-browser-wake', 'cat browser-marker.txt; demi browser tabs --json', 30000),
      model.say('Cloud woke without reviving expired browser tabs'),
    ] })
    expect(woken.received[0]).toContain('exitCode: 0')
    expect(woken.received[0]).toContain('retained')
    expect(woken.received[0]).toContain('"tabs":[]')
    console.info('Cloud idle shutdown and wake preserved files without restoring expired tabs')
  } finally {
    await world.close()
  }
}, 1_200_000)

/** Run a Cloud browser fixture with a localhost website owned by the same shell job. */
function withCloudApplication(command: string): string {
  return `python3 - <<'CLOUD_BROWSER_APPLICATION'
import functools
import http.server
import pathlib
import subprocess
import sys
import threading
root = pathlib.Path('browser-fixture')
(root / 'complete').mkdir(parents=True, exist_ok=True)
(root / 'index.html').write_text('<!doctype html><title>Cloud login</title><form action="/complete/"><label>Email<input name="email"></label><button>Continue</button></form>')
(root / 'complete' / 'index.html').write_text('<!doctype html><title>Cloud browser</title><h1>Cloud signed in</h1>')
handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(root))
with http.server.ThreadingHTTPServer((${JSON.stringify(applicationUrl.hostname)}, ${Number(applicationUrl.port)}), handler) as application:
    serving = threading.Thread(target=application.serve_forever)
    serving.start()
    try:
        result = subprocess.run(['bash', '-c', ${JSON.stringify(command)}])
    finally:
        application.shutdown()
        serving.join()
sys.exit(result.returncode)
CLOUD_BROWSER_APPLICATION`
}
