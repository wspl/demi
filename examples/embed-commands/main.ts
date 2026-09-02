// Embedding Demi's commands in another agent — the loader, a Host, a manifest.
//
//   bun run examples/embed-commands/main.ts
//
// A third party who wants `demi file …` (and their own root beside it) needs
// three things: the command trees, a Host to run `runtime` modules against,
// and the loader. No runner, no tinyjs, no backend. Adding tinybash on top
// gives a whole hostless shell: pipelines and builtins over the same Host,
// root commands through the same loader.
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createDemiCommand } from '@demicodes/coding-agent'
import { buildManifest, createLoader, inMemorySource, rootPaths } from '@demicodes/command-loader'
import { LocalHost } from '@demicodes/host-local'
import { pathArg, runtimeModule, type Command, type DispatchIO } from '@demicodes/shell'
import { runTinybash } from '@demicodes/tinybash'
import { emptyByteStream } from '@demicodes/utils'
import { z } from 'zod'
import helloModule from './hello.command' with { type: 'text' }

/** The embedder's own root, declared with the same tree types as `demi`. */
function scoutRoot(): Command {
  return {
    name: 'scout',
    summary: 'The embedding agent\'s own commands.',
    subcommands: [
      {
        name: 'hello',
        kind: 'runtime',
        module: runtimeModule(helloModule),
        summary: 'Write a greeting file.',
        input: { path: pathArg(z.string().describe('Where to write')), name: z.string().optional().describe('Whom to greet') },
        positionals: ['path'],
      },
    ],
  }
}

async function main(): Promise<void> {
  const dir = mkdtempSync(join(tmpdir(), 'embed-commands-'))
  try {
    // The Host: any implementation of the contract. A real directory here;
    // a store-backed or in-memory Host would do the same.
    const host = new LocalHost(dir, { storeRoot: join(dir, '.store') })

    // The manifest: the trees with JSON Schema, each runtime module transpiled
    // and stored under its hash. The embedder brings the transpiler.
    const transpiler = new Bun.Transpiler({ loader: 'ts', target: 'browser' })
    const manifest = await buildManifest([createDemiCommand(), scoutRoot()], { transpile: (source) => transpiler.transformSync(source) })
    console.log(`manifest ${manifest.hash.slice(0, 12)}: roots ${Object.keys(manifest.roots).join(', ')}, ${Object.keys(manifest.modules).length} modules`)

    // The loader: runtime modules run here against `host`; without an rpc
    // transport, rpc leaves (demi todo) report that they need one.
    const loader = await createLoader({ source: inMemorySource(manifest), host })
    const io = (): DispatchIO => ({ stdin: emptyByteStream(), stdout: (data) => void process.stdout.write(data), stderr: (data) => void process.stderr.write(data), cwd: dir, env: {} })
    await loader.dispatch('scout', ['hello', 'greeting.txt', '--name', 'embedder'], io())
    await loader.dispatch('demi', ['file', 'read', 'greeting.txt'], io())
    await loader.dispatch('demi', ['todo', 'list'], io())

    // A hostless shell on top: tinybash runs the script, builtins over the
    // Host, roots through the loader. `rootPaths` tells tinybash which
    // arguments are paths so it can keep the script inside the namespace.
    const state = { cwd: dir, home: dir, vars: { HOME: dir } }
    const run = async (script: string) => {
      const result = await runTinybash({
        script,
        roots: rootPaths(loader.roots),
        namespace: [dir],
        dispatch: loader.dispatch,
        fs: host.fs,
        state,
        io: { stdout: (data) => void process.stdout.write(data), stderr: (data) => void process.stderr.write(data) },
        identity: { user: 'demi', group: 'demi' },
      })
      console.log(result.kind === 'ran' ? `→ exit ${result.exitCode}` : `→ outside, nothing ran: ${result.message}`)
    }
    await run("demi file create notes.md <<'EOF'\nalpha\nbeta\nEOF\ndemi file read notes.md | grep -n a | sort -r")
    // Parse-first: a script that would leave the namespace is refused whole,
    // before its first statement runs — the embedder hands it to a machine.
    await run('echo touched > marker.txt; cat /etc/passwd')
    console.log(`marker.txt exists: ${await host.fs.exists('marker.txt', { cwd: dir })}`)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}

void main()
