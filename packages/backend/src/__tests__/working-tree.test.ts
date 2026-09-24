import { readdirSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, expect, test } from 'bun:test'
import { startRunner, type Runner } from '@demicodes/host-remote/testing'
import { MAX_MESSAGE_BYTES } from '@demicodes/runner-protocol'
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

/** The runners and directories a test made, which go after it. */
const made: { runners: Runner[]; directories: string[] } = { runners: [], directories: [] }
afterEach(async () => {
  for (const runner of made.runners.splice(0))
    await runner.stop()
  for (const directory of made.directories.splice(0))
    await rm(directory, { recursive: true, force: true })
})

/** A conversation whose execution directory is `runnerDir` on a paired device running a real runner. */
async function pairedConversation() {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-wt-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-wt-state-'))
  const runnerDir = await mkdtemp(join(tmpdir(), 'demi-wt-runner-'))
  made.directories.push(dataDir, stateDir, runnerDir)
  const backend = await openBackend({ dataDir, port: 0, runner: { pingIntervalMs: 0 } })
  const runner = await startRunner({ backendUrl: backend.url, stateDir, home: runnerDir, name: 'test-device' })
  made.runners.push(runner)
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
  return { backend, runner, runnerDir, conversation }
}

test(
  'the change routes list the uncommitted changes of the execution directory and read one file',
  async () => {
    const { backend, runner, runnerDir, conversation } = await pairedConversation()

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
      files: Array<{ path: string; status: string; kind: string; added: number; removed: number }>;
      truncated: boolean;
      watched: boolean
    }
    expect(changes.root).toBe(runnerDir)
    expect(changes.repository).toBe(true)
    expect(changes.head).toMatch(/^[0-9a-f]{40}$/)
    expect(changes.truncated).toBe(false)
    expect(changes.files).toEqual([
      { path: 'a.txt', status: ' M', kind: 'modified', added: 1, removed: 0 },
      { path: 'b.txt', status: '??', kind: 'added', added: 1, removed: 0 },
      { path: 'blob.bin', status: '??', kind: 'added', added: 0, removed: 0 },
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

    // A listing over the runner's message limit fails that request alone.
    const crowded = join(runnerDir, 'crowded')
    await mkdir(crowded)
    const name = 'n'.repeat(200)
    for (let index = 0; index <= MAX_MESSAGE_BYTES / 200; index += 1)
      await writeFile(join(crowded, `${name}${index}`), '')
    const listing = await api(backend, `/api/conversations/${conversation.id}/fs?path=${encodeURIComponent(crowded)}`)
    expect(listing.status).toBe(413)
    expect(((await listing.json()) as { code: string }).code).toBe('directory_too_large')
    const after = await api(backend, `/api/conversations/${conversation.id}/fs/file?path=${encodeURIComponent(join(runnerDir, 'a.txt'))}`)
    expect(after.status).toBe(200)

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

/** Bytes that differ at every position, so a misplaced range shows. */
function pattern(size: number): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(size)
  for (let index = 0; index < size; index += 1)
    bytes[index] = (index * 31 + (index >> 8)) % 251
  return bytes
}

test(
  'the raw routes stream a file by range, under headers that keep it inert, and the committed side from git',
  async () => {
    const { backend, runnerDir, conversation } = await pairedConversation()
    const raw = (path: string, query: Record<string, string> = {}, init?: RequestInit) => api(
      backend,
      `/api/conversations/${conversation.id}/fs/raw?${new URLSearchParams({ path: join(runnerDir, path), ...query })}`,
      init,
    )
    const image = pattern(300_000)
    await writeFile(join(runnerDir, 'logo.svg'), image)

    const whole = await raw('logo.svg')
    expect(whole.status).toBe(200)
    expect(whole.headers.get('content-type')).toBe('image/svg+xml')
    expect(whole.headers.get('content-security-policy')).toBe("default-src 'none'; style-src 'unsafe-inline'; sandbox")
    expect(whole.headers.get('x-content-type-options')).toBe('nosniff')
    expect(whole.headers.get('x-accel-buffering')).toBe('no')
    expect(whole.headers.get('cache-control')).toBe('private, no-cache')
    expect(whole.headers.get('accept-ranges')).toBe('bytes')
    const etag = whole.headers.get('etag')!
    expect(etag).toMatch(/^W\/"[0-9a-f]+-[0-9a-f]+"$/)
    expect(new Uint8Array(await whole.arrayBuffer())).toEqual(image)

    const head = await raw('logo.svg', {}, { method: 'HEAD' })
    expect(head.status).toBe(200)
    expect(head.headers.get('content-length')).toBe(String(image.length))
    expect(head.headers.get('etag')).toBe(etag)
    expect(head.headers.get('last-modified')).not.toBeNull()

    const part = await raw('logo.svg', {}, { headers: { range: 'bytes=1000-1999' } })
    expect(part.status).toBe(206)
    expect(part.headers.get('content-range')).toBe(`bytes 1000-1999/${image.length}`)
    expect(new Uint8Array(await part.arrayBuffer())).toEqual(image.subarray(1000, 2000))
    expect((await raw('logo.svg', {}, { headers: { range: `bytes=${image.length}-` } })).status).toBe(416)

    expect((await raw('logo.svg', {}, { headers: { 'if-none-match': etag } })).status).toBe(304)
    expect((await raw('logo.svg', { version: etag })).status).toBe(200)
    await writeFile(join(runnerDir, 'logo.svg'), pattern(10))
    const changed = await raw('logo.svg', { version: etag })
    expect(changed.status).toBe(412)
    expect(((await changed.json()) as { code: string }).code).toBe('file_changed')

    const download = await raw('logo.svg', { download: 'true' })
    expect(download.headers.get('content-type')).toBe('application/octet-stream')
    expect(download.headers.get('content-disposition')).toStartWith('attachment; filename="logo.svg"')
    expect(download.headers.get('content-security-policy')).toBeNull()
    await download.arrayBuffer()
    await writeFile(join(runnerDir, 'page.html'), '<script>alert(1)</script>')
    const html = await raw('page.html')
    expect(html.headers.get('content-type')).toBe('application/octet-stream')
    expect(html.headers.get('content-disposition')).toStartWith('attachment')
    await html.arrayBuffer()
    expect((await raw('.')).status).toBe(404)
    expect((await raw('missing.png')).status).toBe(404)
    expect((await raw('logo.svg', { download: '1' })).status).toBe(400)

    // Far past the runner's message limit, whole and in order.
    const video = pattern(3 * MAX_MESSAGE_BYTES + 5)
    await writeFile(join(runnerDir, 'demo.mp4'), video)
    const streamed = await raw('demo.mp4')
    expect(streamed.headers.get('content-type')).toBe('video/mp4')
    // Streamed, the answer has no length; HEAD reports it.
    expect(streamed.headers.get('content-length')).toBeNull()
    expect(streamed.headers.get('content-security-policy')).toBeNull()
    expect(new Uint8Array(await streamed.arrayBuffer())).toEqual(video)

    // The committed side of a change comes from git, by range as well.
    git(runnerDir, 'init', '-q', '-b', 'main')
    const committed = pattern(5_000)
    await writeFile(join(runnerDir, 'chart.png'), committed)
    git(runnerDir, 'add', 'chart.png')
    git(runnerDir, 'commit', '-q', '-m', 'chart')
    await writeFile(join(runnerDir, 'chart.png'), pattern(7))
    const original = (query: string, init?: RequestInit) =>
      api(backend, `/api/conversations/${conversation.id}/changes/raw?${query}`, init)
    const before = await original('path=chart.png')
    expect(before.status).toBe(200)
    expect(before.headers.get('content-type')).toBe('image/png')
    expect(new Uint8Array(await before.arrayBuffer())).toEqual(committed)
    const tail = await original('path=chart.png', { headers: { range: 'bytes=-100' } })
    expect(tail.status).toBe(206)
    expect(new Uint8Array(await tail.arrayBuffer())).toEqual(committed.subarray(committed.length - 100))
    expect((await original('path=chart.png', { method: 'HEAD' })).headers.get('content-length')).toBe('5000')
    const saved = await original('path=chart.png&download=true')
    expect(saved.headers.get('content-type')).toBe('application/octet-stream')
    expect(saved.headers.get('content-disposition')).toStartWith('attachment; filename="chart.png"')
    expect(new Uint8Array(await saved.arrayBuffer())).toEqual(committed)
    expect((await original('path=new.png')).status).toBe(404)
    expect((await original('path=../escape.png')).status).toBe(400)

    // Git's copy is decoded whole, so one over the runner's 8 MiB is refused.
    await writeFile(join(runnerDir, 'poster.png'), pattern(8 * 1024 * 1024 + 1))
    git(runnerDir, 'add', 'poster.png')
    git(runnerDir, 'commit', '-q', '-m', 'poster')
    const oversized = await original('path=poster.png')
    expect(oversized.status).toBe(413)
    expect(((await oversized.json()) as { code: string }).code).toBe('file_too_large')
    expect((await original('path=poster.png', { method: 'HEAD' })).status).toBe(413)

    await backend.close()
  },
  60_000
)

test(
  'archiving a conversation ends its open transfers instead of waiting for them',
  async () => {
    const { backend, runnerDir, conversation } = await pairedConversation()
    await writeFile(join(runnerDir, 'long.mp4'), pattern(64 * 1024 * 1024))
    const url = `/api/conversations/${conversation.id}/fs/raw?${new URLSearchParams({ path: join(runnerDir, 'long.mp4') })}`
    const playing = await api(backend, url)
    expect(playing.status).toBe(200)
    const reader = playing.body!.getReader()
    await reader.read()

    const archived = await api(backend, `/api/conversations/${conversation.id}`, {
      method: 'PATCH',
      body: JSON.stringify({ archived: true }),
      headers: { 'content-type': 'application/json' },
    })
    expect(archived.status).toBe(200)
    // The transfer was cut short, never completed.
    let ended: 'complete' | 'cut' = 'complete'
    try {
      for (;;) {
        if ((await reader.read()).done)
          break
      }
    } catch {
      ended = 'cut'
    }
    expect(ended).toBe('cut')
    const after = await api(backend, url)
    expect(after.status).toBe(409)
    expect(((await after.json()) as { code: string }).code).toBe('conversation_archived')

    await backend.close()
  },
  60_000
)

/** `size` bytes of `pattern`, streamed a MiB at a time without holding them. */
function streamedPattern(size: number): ReadableStream<Uint8Array> {
  const block = pattern(1024 * 1024)
  let sent = 0
  return new ReadableStream({
    pull(controller) {
      if (sent >= size) {
        controller.close()
        return
      }
      const chunk = block.subarray(0, Math.min(block.length, size - sent))
      controller.enqueue(chunk)
      sent += chunk.length
    },
  })
}

test(
  'an upload streams into place whole, asks before it writes over a file, and never over a directory',
  async () => {
    const { backend, runnerDir, conversation } = await pairedConversation()
    const put = (path: string, body: RequestInit['body'], query: Record<string, string> = {}) => api(
      backend,
      `/api/conversations/${conversation.id}/fs/raw?${new URLSearchParams({ path: join(runnerDir, path), ...query })}`,
      { method: 'PUT', body, ...(body instanceof ReadableStream ? { duplex: 'half' } : {}) } as RequestInit,
    )
    const code = async (response: Response) => ((await response.json()) as { code: string }).code

    expect((await put('notes.md', 'first')).status).toBe(204)
    expect(await readFile(join(runnerDir, 'notes.md'), 'utf8')).toBe('first')
    const taken = await put('notes.md', 'second')
    expect(taken.status).toBe(409)
    expect(await code(taken)).toBe('file_exists')
    expect(await readFile(join(runnerDir, 'notes.md'), 'utf8')).toBe('first')
    expect((await put('notes.md', 'second', { replace: 'true' })).status).toBe(204)
    expect(await readFile(join(runnerDir, 'notes.md'), 'utf8')).toBe('second')
    expect((await put('empty.txt', null)).status).toBe(204)
    expect((await stat(join(runnerDir, 'empty.txt'))).size).toBe(0)

    await mkdir(join(runnerDir, 'docs'))
    const directory = await put('docs', 'x', { replace: 'true' })
    expect(directory.status).toBe(409)
    expect(await code(directory)).toBe('is_directory')
    expect((await put('missing/file.txt', 'x')).status).toBe(404)
    expect((await put('notes.md', 'x', { replace: 'yes' })).status).toBe(400)

    // Past the 128 MiB a server caps a body at by default, streamed through
    // the runner a chunk at a time.
    const size = 130 * 1024 * 1024
    expect((await put('large.bin', streamedPattern(size))).status).toBe(204)
    expect((await stat(join(runnerDir, 'large.bin'))).size).toBe(size)
    const tail = (await readFile(join(runnerDir, 'large.bin'))).subarray(size - 1024 * 1024)
    expect(new Uint8Array(tail)).toEqual(pattern(1024 * 1024))

    // A body cut short leaves the path as it was, with no partial copy beside it.
    const broken = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(pattern(1024 * 1024))
        setTimeout(() => controller.error(new Error('the browser went away')), 50)
      },
    })
    await put('notes.md', broken, { replace: 'true' }).catch(() => null)
    await waitFor(() => readdirSync(runnerDir).every((name) => !name.startsWith('.demi-write-')))
    expect(await readFile(join(runnerDir, 'notes.md'), 'utf8')).toBe('second')

    await backend.close()
  },
  120_000
)

test(
  'a delete takes a file or a directory with what is in it, and refuses the directories the host needs',
  async () => {
    const { backend, runnerDir, conversation } = await pairedConversation()
    const remove = (path: string) => api(
      backend,
      `/api/conversations/${conversation.id}/fs?${new URLSearchParams({ path })}`,
      { method: 'DELETE' },
    )
    await mkdir(join(runnerDir, 'photos', '2024'), { recursive: true })
    await writeFile(join(runnerDir, 'photos', '2024', 'a.jpg'), 'a')
    await writeFile(join(runnerDir, 'notes.md'), 'n')

    expect((await remove(join(runnerDir, 'notes.md'))).status).toBe(204)
    expect((await remove(join(runnerDir, 'photos'))).status).toBe(204)
    expect((await remove(join(runnerDir, 'nothing'))).status).toBe(204)
    expect(await readdir(runnerDir)).toEqual([])

    for (const path of [runnerDir, join(runnerDir, '..'), '/', runnerDir.toUpperCase()]) {
      const refused = await remove(path)
      expect(refused.status).toBe(409)
      expect(((await refused.json()) as { code: string }).code).toBe('protected_path')
    }
    expect((await remove('photos')).status).toBe(400)

    await backend.close()
  },
  60_000
)
