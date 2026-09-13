import { nativePackageFixture } from '@demicodes/demi-package/testing'
import { runnerBinary } from '../src/testing'

// Prepare both executables before Bun starts per-test timeouts.
console.log(`Test runner: ${await runnerBinary()}`)
await nativePackageFixture()
