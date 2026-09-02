// The Host conformance suite on tinyjs: the definition of done for the
// machine layer. Bundled by the Bun test and run as `tinyjs main.mjs`.
import { env, exit, openHandles } from 'tinyjs:runtime'
import { hostConformanceCases } from '@demicodes/shell/testing'
import { createRunnerHost } from '../../machine'

const root = env.HOST_CONFORMANCE_ROOT
if (!root) {
  console.error('HOST_CONFORMANCE_ROOT is not set')
  exit(2)
}
const host = createRunnerHost({ defaultCwd: root, commandArtifactsDir: `${root}/output`, storeDir: `${root}/store` })
const cases = hostConformanceCases({ host, root, path: env.PATH })
let failed = 0
for (const conformance of cases) {
  const started = performance.now()
  try {
    await conformance.run()
    console.log(`ok   ${conformance.name} (${(performance.now() - started).toFixed(1)}ms)`)
  } catch (error) {
    failed += 1
    const detail = error instanceof Error ? `${error.name}: ${error.message}\n${error.stack ?? ''}` : String(error)
    console.log(`FAIL ${conformance.name}\n     ${detail.trimEnd().replace(/\n/g, '\n     ')}`)
  }
}
const open = openHandles()
if (open !== 0) {
  failed += 1
  console.log(`FAIL ${open} handle(s) still open after the suite`)
}
console.log(`\n${cases.length - failed}/${cases.length} passed`)
exit(failed === 0 ? 0 : 1)
