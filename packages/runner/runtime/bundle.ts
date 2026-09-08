import { resolve } from 'node:path'

/** Isolated bundler process: embed the worker source into the runner module. */
export async function bundleRuntime(entry: string, output: string): Promise<void> {
  const worker = await Bun.build({ entrypoints: [resolve(import.meta.dir, '../src/commands/worker.ts')], target: 'browser', format: 'esm', conditions: ['development'], external: ['tjs:*'] })
  if (!worker.success) throw new Error(worker.logs.join('\n'))
  const source = await worker.outputs[0]!.text()
  const app = await Bun.build({ entrypoints: [entry], target: 'browser', format: 'esm', conditions: ['development'], external: ['tjs:*'], define: { DEMI_COMMAND_WORKER_SOURCE: JSON.stringify(source) } })
  if (!app.success) throw new Error(app.logs.join('\n'))
  await Bun.write(output, app.outputs[0]!)
}
if (import.meta.main) {
  const [entry, output] = process.argv.slice(2)
  if (!entry || !output) throw new Error('Usage: bundle.ts entry output')
  await bundleRuntime(entry, output)
}
