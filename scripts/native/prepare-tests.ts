import { nativePackageFixture, runnerBinary } from '@demicodes/host-remote/testing'

// Prepare both executables before Bun starts per-test timeouts.
console.log(`Test runner: ${await runnerBinary()}`)
await nativePackageFixture()
