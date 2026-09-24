import { expect, test } from 'bun:test'
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { BashEnvironment, createLogicalHostCwd } from '@demicodes/shell'
import { LocalHost } from '../local-host'

test('read preserves UTF-8 JSONL through real child process stdin and argv', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-read-utf8-'))
  try {
    const content = ['任务600578金币发放计数下降 🙂 café Ã©'].map(title => JSON.stringify({ title })).join('\n') + '\n'
    await writeFile(join(root, 'input'), content)
    const host = new LocalHost(root, { storeRoot: join(root, 'store') })
    // Exercise byte transport independently of platform directory-fd spawning.
    host.process.openCwd = async path => createLogicalHostCwd(path)
    const shell = new BashEnvironment({ host })
    const result = await shell.exec({
      script: 'while read -r line; do echo "$line" | /bin/cat >> piped; /usr/bin/printf "%s\\n" "$line" >> args; done < input',
      timeoutMs: 10000,
    })
    expect(result.status).toBe('exited')
    if (result.status !== 'exited') throw new Error('Expected command to exit')
    expect(result.stderr?.tail).toBe('')
    expect(result.exitCode).toBe(0)
    expect(await readFile(join(root, 'piped'), 'utf8')).toBe(content)
    expect(await readFile(join(root, 'args'), 'utf8')).toBe(content)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
