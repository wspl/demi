import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { startRunner } from '@demicodes/host-remote/testing'
import { delay, waitFor } from '@demicodes/utils'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { openBackend, type TestBackend } from './session'

// The working-tree routes (`web-api.md` § File text and working tree changes)
// over a paired device: the change list, one file's two sides, and file text.

async function api(backend: TestBackend, path: string, init?: RequestInit): Promise<Response> {
  return backend.session.fetch(path, init)
}

function git(cwd: string, ...args: string[]): void {
  const result = Bun.spawnSync(['git', ...args], {
    cwd,
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: 'Test',
      GIT_AUTHOR_EMAIL: 'test@example.com',
      GIT_COMMITTER_NAME: 'Test',
      GIT_COMMITTER_EMAIL: 'test@example.com',
    },
  })
  expect(result.exitCode).toBe(0)
}

test(
  'the change routes list the uncommitted changes of the execution directory and read one file',
  async () => {
    const dataDir = await mkdtemp(join(tmpdir(), 'demi-wt-'))
    const stateDir = await mkdtemp(join(tmpdir(), 'demi-wt-state-'))
    const runnerDir = await mkdtemp(join(tmpdir(), 'demi-wt-runner-'))
    const backend = await openBackend({ dataDir, port: 0, runner: { pingIntervalMs: 0 } })
    const runner = await startRunner({ backendUrl: backend.url, stateDir, home: runnerDir, name: 'test-device' })
    await waitFor(() => runner.codes.length > 0, undefined, { timeoutMs: 5_000 })
    const claimed = await api(backend, '/api/devices/claim', {
      method: 'POST',
      body: JSON.stringify({ code: runner.codes[0] }),
      headers: { 'content-type': 'application/json' },
    })
    const { device } = (await claimed.json()) as { device: { id: string } }
    await waitFor(() => runner.statuses.includes('online'))

    const created = await api(backend, '/api/conversations', {
      method: 'POST',
      body: JSON.stringify({ id: crypto.randomUUID() }),
    })
    const { conversation } = (await created.json()) as { conversation: { id: string } }
    const controlDb = openSqliteDatabase(join(dataDir, 'control.sqlite'))
    const control = new LocalControlService(controlDb)
    const workspace = await control.createWorkspace({
      userId: backend.session.user.id,
      deviceId: device.id,
      path: runnerDir,
      name: 'test workspace',
    })
    await control.setConversationWorkspace(conversation.id, workspace.id)
    controlDb.close()

    // Not a repository yet: an answer, not an error.
    const outside = (await (await api(backend, `/api/conversations/${conversation.id}/changes`)).json()) as {
      root: string;
      repository: boolean;
      files: unknown[]
    }
    expect(outside).toMatchObject({ root: runnerDir, repository: false, files: [] })

    git(runnerDir, 'init', '-q', '-b', 'main')
    await writeFile(join(runnerDir, 'a.txt'), '1\n2\n')
    git(runnerDir, 'add', '.')
    git(runnerDir, 'commit', '-q', '-m', 'first')
    await writeFile(join(runnerDir, 'a.txt'), '1\n2\n3\n')
    await writeFile(join(runnerDir, 'b.txt'), 'new\n')
    await writeFile(join(runnerDir, 'blob.bin'), new Uint8Array([0, 255, 1]))

    const changes = (await (await api(backend, `/api/conversations/${conversation.id}/changes`)).json()) as {
      root: string;
      repository: boolean;
      head: string | null;
      files: Array<{ path: string; kind: string; added: number; removed: number }>;
      truncated: boolean;
      watched: boolean
    }
    expect(changes.root).toBe(runnerDir)
    expect(changes.repository).toBe(true)
    expect(changes.head).toMatch(/^[0-9a-f]{40}$/)
    expect(changes.truncated).toBe(false)
    expect(changes.files).toEqual([
      { path: 'a.txt', kind: 'modified', added: 1, removed: 0 },
      { path: 'b.txt', kind: 'added', added: 1, removed: 0 },
      { path: 'blob.bin', kind: 'added', added: 0, removed: 0 },
    ])

    const modified = await api(backend, `/api/conversations/${conversation.id}/changes/file?path=a.txt`)
    expect(await modified.json()).toEqual({ original: '1\n2\n', modified: '1\n2\n3\n' })
    const added = await api(backend, `/api/conversations/${conversation.id}/changes/file?path=b.txt`)
    expect(await added.json()).toEqual({ original: '', modified: 'new\n' })
    const binary = await api(backend, `/api/conversations/${conversation.id}/changes/file?path=blob.bin`)
    expect(binary.status).toBe(415)
    const escaping = await api(backend, `/api/conversations/${conversation.id}/changes/file?path=../x`)
    expect(escaping.status).toBe(400)

    const text = await api(backend, `/api/conversations/${conversation.id}/fs/file?path=${encodeURIComponent(join(runnerDir, 'a.txt'))}`)
    expect(await text.json()).toEqual({ path: join(runnerDir, 'a.txt'), text: '1\n2\n3\n' })
    const missing = await api(backend, `/api/conversations/${conversation.id}/fs/file?path=${encodeURIComponent(join(runnerDir, 'nope'))}`)
    expect(missing.status).toBe(404)

    // Offline: the routes say so rather than waking anything.
    await runner.stop()
    let offline = false
    for (let tries = 0; tries < 100 && !offline; tries += 1) {
      const rows = (await (await api(backend, '/api/devices')).json()) as { devices: Array<{ online: boolean }> }
      offline = rows.devices[0]?.online === false
      if (!offline)
        await delay(20)
    }
    expect(offline).toBe(true)
    const refused = await api(backend, `/api/conversations/${conversation.id}/changes`)
    expect(refused.status).toBe(409)
    expect(((await refused.json()) as { code: string }).code).toBe('device_offline')

    await backend.close()
  },
  30_000
)
