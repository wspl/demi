import { mkdir, writeFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { msgpackCodec } from '../src/codec'
import { backendToRunnerMessageSchema } from '../src/schemas'

const dir = resolve(import.meta.dir, '../src/fixtures')
await mkdir(dir, { recursive: true })
const fixtures = {
  binary: { type: 'job_stdin', jobId: 'job', bytes: new Uint8Array([0, 255, 13, 10]) },
  time: { type: 'fs_utimes', id: 'time', path: '/tmp/data', atime: new Date(-123456789), mtime: new Date(1700000000123) },
  job: {
    type: 'job_start',
    jobId: 'job',
    context: {
      conversation: 'conversation',
      caller: { kind: 'agent', node: 'node' },
      locale: { timeZone: 'Asia/Shanghai', languages: ['zh-CN', 'en'] },
    },
    script: 'printf hello',
    cwd: '/tmp',
    env: { TEST: 'yes' },
  },
}
for (const [name, value] of Object.entries(fixtures)) {
  backendToRunnerMessageSchema.parse(value)
  await writeFile(resolve(dir, `${name}.msgpack`), msgpackCodec.encode(value))
}
