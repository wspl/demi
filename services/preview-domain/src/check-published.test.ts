import { afterEach, beforeEach, expect, test } from 'bun:test'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

// The deployment's check of published versions (`scripts/check-published.sh`),
// run in a scratch repository: a push may add a file to a published version,
// and may not change, remove or rename one. About 3.5 s, a dozen git
// commands in a new repository.

const CHECK = resolve(import.meta.dir, '../scripts/check-published.sh')
const VERSION = 'services/preview-domain/static/__demi/v1'
let repository: string

function git(...args: string[]): string {
  const run = Bun.spawnSync(['git', '-c', 'user.name=Test', '-c', 'user.email=test@example.test', ...args], { cwd: repository })
  if (run.exitCode !== 0) {
    throw new Error(`git ${args.join(' ')}: ${run.stderr.toString()}`)
  }
  return run.stdout.toString().trim()
}

async function file(path: string, text: string): Promise<void> {
  await mkdir(join(repository, path, '..'), { recursive: true })
  await writeFile(join(repository, path), text)
}

/** Commits what the test changed, and runs the check from `before` to it. */
function check(before: string): { passed: boolean; error: string } {
  git('add', '-A')
  git('commit', '-q', '--allow-empty', '-m', 'push')
  const run = Bun.spawnSync([CHECK, before], { cwd: repository })
  return { passed: run.exitCode === 0, error: run.stderr.toString() }
}

beforeEach(async () => {
  repository = await mkdtemp(join(tmpdir(), 'check-published-'))
  git('init', '-q')
  await file(`${VERSION}/boot.js`, 'boot')
  await file(`${VERSION}/sw.js`, 'forwarder')
  git('add', '-A')
  git('commit', '-q', '-m', 'v1 published')
})

afterEach(async () => {
  await rm(repository, { recursive: true, force: true })
})

test('a new file in a published version, or a new version, passes', async () => {
  const before = git('rev-parse', 'HEAD')
  await file(`${VERSION}/state.js`, 'codec')
  await file('services/preview-domain/static/__demi/v2/boot.js', 'boot 2')
  expect(check(before)).toEqual({ passed: true, error: '' })
})

test('a changed, removed or renamed file of a published version stops the deployment', async () => {
  const before = git('rev-parse', 'HEAD')
  await file(`${VERSION}/boot.js`, 'boot changed')
  expect(check(before)).toMatchObject({ passed: false, error: expect.stringContaining(`M\t${VERSION}/boot.js`) })

  const changed = git('rev-parse', 'HEAD')
  git('rm', '-q', `${VERSION}/sw.js`)
  expect(check(changed)).toMatchObject({ passed: false, error: expect.stringContaining(`D\t${VERSION}/sw.js`) })

  const removed = git('rev-parse', 'HEAD')
  git('mv', `${VERSION}/boot.js`, `${VERSION}/start.js`)
  expect(check(removed)).toMatchObject({ passed: false, error: expect.stringContaining(`D\t${VERSION}/boot.js`) })
})
