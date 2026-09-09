import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { openBackend } from './session'

const json = (body: unknown, method = 'POST') => ({ method, body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })

test('state revalidation changes after settings or conversation mutations and excludes secrets', async () => {
  const backend = await openBackend({ dataDir: await mkdtemp(join(tmpdir(), 'demi-state-')), port: 0 })
  try {
    const first = await backend.session.fetch('/api/state')
    expect(first.status).toBe(200)
    const etag = first.headers.get('etag')!
    expect((await backend.session.fetch('/api/state', { headers: { 'If-None-Match': etag } })).status).toBe(304)
    await backend.session.fetch('/api/settings/preferences', json({ appearance: { theme: 'dark' } }, 'PATCH'))
    const updated = await backend.session.fetch('/api/state', { headers: { 'If-None-Match': etag } })
    expect(updated.status).toBe(200)
    expect(await updated.json()).toMatchObject({ preferences: { appearance: { theme: 'dark' } } })
    await backend.session.fetch('/api/providers', json({ providerType: 'openai', label: 'API', apiKey: 'fixture-secret-key' }))
    const state = await (await backend.session.fetch('/api/state')).text()
    expect(state).not.toContain('fixture-secret-key')
    expect(state).not.toContain('passwordHash')
    expect((await fetch(`${backend.url}/api/state`)).status).toBe(401)
  } finally {
    await backend.close()
  }
})

test('optional SPA hosting serves assets and deep navigation while preserving API errors', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-web-assets-'))
  const webDirectory = join(dataDir, 'web')
  await mkdir(webDirectory)
  await writeFile(join(webDirectory, 'index.html'), '<html>fixture page</html>')
  await writeFile(join(webDirectory, 'main.js'), 'export const fixture = true')
  const backend = await openBackend({ dataDir, port: 0, webDirectory })
  try {
    expect(await (await fetch(`${backend.url}/conversation/example`, { headers: { Accept: 'text/html' } })).text()).toContain('fixture page')
    expect(await (await fetch(`${backend.url}/main.js`)).text()).toContain('fixture = true')
    expect((await backend.session.fetch('/api/no-such-resource', { headers: { Accept: 'text/html' } })).status).toBe(404)
    expect((await fetch(`${backend.url}/missing.js`, { headers: { Accept: 'text/html' } })).status).toBe(404)
  } finally {
    await backend.close()
  }
})
