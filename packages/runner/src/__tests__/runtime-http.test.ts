import { expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { deferred } from '@demicodes/utils'
import { txikiBinary } from '../testing'

// The native HTTP client must not apply its connection deadline to body
// production or a server's response. All requests share one delayed release.
test('native fetch permits quiet uploads and delayed replies while explicit abort remains available', async () => {
  const binary = txikiBinary()
  const work = await mkdtemp(join(tmpdir(), 'demi-http-runtime-'))
  const released = deferred<void>()
  const connected = new Set<string>()
  const server = Bun.serve({
    port: 0,
    idleTimeout: 0,
    async fetch(request) {
      const path = new URL(request.url).pathname
      connected.add(path)
      if (path === '/echo')
        return new Response(await request.text())
      if (path === '/stream') {
        return new Response(new ReadableStream({
          async start(controller) {
            controller.enqueue(new TextEncoder().encode('first'))
            await released.promise
            controller.enqueue(new TextEncoder().encode('last'))
            controller.close()
          }
        }))
      }
      const body = await request.text()
      await released.promise
      return new Response(body || 'reply')
    }
  })
  const entry = join(work, 'entry.js')
  await Bun.write(entry, `
const url = 'http://127.0.0.1:${server.port}';
const encode = text => new TextEncoder().encode(text);
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const body = new ReadableStream({
  async start(controller) {
    controller.enqueue(encode('first'));
    await sleep(25000);
    controller.enqueue(encode('last'));
    controller.close();
  }
});
const single = new ReadableStream({ start(controller) {
  controller.enqueue(encode('upload'));
  controller.close();
} });
const abort = new AbortController();
const cancelled = fetch(url + '/abort', { signal: abort.signal }).then(
  () => { throw new Error('expected abort'); },
  error => { if (error.name !== 'AbortError') throw error; }
);
setTimeout(() => abort.abort(), 100);
const timedOut = new Promise((resolve, reject) => {
  const request = new XMLHttpRequest();
  request.open('GET', url + '/timeout');
  request.timeout = 100;
  request.ontimeout = resolve;
  request.onload = () => reject(new Error('expected explicit timeout'));
  request.onerror = () => reject(new Error('unexpected network error'));
  request.send();
});
const responses = await Promise.all([
  fetch(url + '/reply').catch(error => { throw new Error('reply: ' + error.message); }),
  fetch(url + '/after-upload', { method: 'PUT', body: single, duplex: 'half' }).catch(error => { throw new Error('upload reply: ' + error.message); }),
  fetch(url + '/after-fixed', { method: 'POST', body: 'fixed' }).catch(error => { throw new Error('fixed reply: ' + error.message); }),
  fetch(url + '/echo', { method: 'PUT', body, duplex: 'half' }).catch(error => { throw new Error('echo: ' + error.message); }),
  fetch(url + '/stream'),
]);
console.log(JSON.stringify(await Promise.all(responses.map(response => response.text()))));
await Promise.all([cancelled, timedOut]);
`)
  const releaseTimer = setTimeout(() => released.resolve(), 25_000)
  const child = Bun.spawn([binary, 'run', entry], { stdout: 'pipe', stderr: 'pipe' })
  const deadline = setTimeout(() => child.kill(), 35_000)
  try {
    const [exit, stdout, stderr] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ])
    expect(exit, stderr).toBe(0)
    expect(JSON.parse(stdout)).toEqual(['reply', 'upload', 'fixed', 'firstlast', 'firstlast'])
    expect(connected.size).toBe(7)
  } finally {
    clearTimeout(releaseTimer)
    clearTimeout(deadline)
    released.resolve()
    child.kill()
    await child.exited
    server.stop(true)
    await rm(work, { recursive: true, force: true })
  }
}, 60_000)
