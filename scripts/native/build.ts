import { mkdir, readFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { existsSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { parseArgs } from 'node:util'
import { z } from 'zod'
import { selectedTargets } from './release-files'
import { NATIVE_PACKAGE_CRATES } from './package-info'

const APPLE_SDK_VERSION = '15.4'
const WINDOWS_SDK_VERSION = '10.0.26100'
const WINDOWS_CRT_VERSION = '14.44.17.14'
const MACOS_MINIMUM = '13.0'

const { values } = parseArgs({ options: {
  artifacts: { type: 'string', default: '.cache/native-target' },
  target: { type: 'string', multiple: true },
  package: { type: 'string', multiple: true },
  container: { type: 'string' },
  sdk: { type: 'string' },
} })
const root = resolve(import.meta.dir, '../..')
const artifacts = resolve(root, values.artifacts)
const targets = selectedTargets(values.target)
// The crates named by repeated `--package` options, or the runner and every command package.
const crates = z.array(z.enum(['demi-runner', ...NATIVE_PACKAGE_CRATES]))
  .parse(values.package ?? ['demi-runner', ...NATIVE_PACKAGE_CRATES])
const sdk = values.sdk ? resolve(values.sdk) : process.env.SDKROOT
const cache = join(root, '.cache')
const xwinCache = join(cache, `native-xwin-${WINDOWS_CRT_VERSION}-${WINDOWS_SDK_VERSION}`)
if (targets.some(target => target.includes('apple'))) {
  if (!sdk)
    throw new Error(`Pass --sdk <MacOSX${APPLE_SDK_VERSION}.sdk> for Apple cross-compilation`)
  z.object({ Version: z.literal(APPLE_SDK_VERSION) }).loose()
    .parse(JSON.parse(await readFile(join(sdk, 'SDKSettings.json'), 'utf8')))
}
await mkdir(artifacts, { recursive: true })
if (values.container) {
  await mkdir(join(cache, 'native-registry'), { recursive: true })
  await mkdir(join(cache, 'native-git'), { recursive: true })
}
const installedCargo = join(homedir(), '.cargo/bin', process.platform === 'win32' ? 'cargo.exe' : 'cargo')
const cargo = existsSync(installedCargo) ? installedCargo : 'cargo'
for (const target of targets) {
  const windows = target.includes('windows')
  const macos = target.includes('apple')
  const args = [windows ? 'xwin' : 'zigbuild', ...(windows ? ['build'] : []),
    '--release', '--locked', '--target', target,
    ...crates.flatMap(crate => ['-p', crate])]
  const commonEnv: Record<string, string> = {
    XWIN_SDK_VERSION: WINDOWS_SDK_VERSION,
    XWIN_CRT_VERSION: WINDOWS_CRT_VERSION,
    MACOSX_DEPLOYMENT_TARGET: MACOS_MINIMUM,
    // ring selects `clang` for Windows ARM64. Keep the MSVC driver dialect
    // selected by cargo-xwin, including its SDK include flags and optimization.
    ...(windows ? { CFLAGS: '--driver-mode=cl /O2' } : {}),
  }
  const buildEnv: Record<string, string | undefined> = {
    ...process.env,
    ...commonEnv,
    CARGO_TARGET_DIR: artifacts,
    RUSTFLAGS: windows ? '-C target-feature=+crt-static'
      : macos ? `-L framework=${sdk}/System/Library/Frameworks` : undefined,
    SDKROOT: macos ? sdk : undefined,
    XWIN_CACHE_DIR: xwinCache,
  }
  let command = [cargo, ...args]
  if (values.container) {
    const containerEnv: Record<string, string> = {
      ...commonEnv,
      CARGO_TARGET_DIR: '/build',
      XWIN_CACHE_DIR: xwinCache.replace(root, '/work'),
      CARGO_BUILD_JOBS: '4',
      ZIG_GLOBAL_CACHE_DIR: '/work/.cache/native-zig',
      ...(windows ? { RUSTFLAGS: '-C target-feature=+crt-static' } : {}),
      ...(macos ? {
        SDKROOT: '/sdk',
        RUSTFLAGS: '-L framework=/sdk/System/Library/Frameworks',
      } : {}),
    }
    command = ['docker', 'run', '--rm',
      '-v', `${root}:/work`, '-v', `${artifacts}:/build`,
      '-v', `${join(cache, 'native-registry')}:/usr/local/cargo/registry`,
      '-v', `${join(cache, 'native-git')}:/usr/local/cargo/git`,
      ...(macos ? ['-v', `${sdk}:/sdk:ro`] : []),
      ...Object.entries(containerEnv).flatMap(([key, value]) => ['-e', `${key}=${value}`]),
      values.container, 'cargo', ...args]
  }
  console.log(`Native build: ${target}`)
  const child = Bun.spawn(command, { cwd: root, env: buildEnv, stdout: 'inherit', stderr: 'inherit' })
  if (await child.exited !== 0)
    throw new Error(`Native build failed: ${target}`)
}
console.log(`Native artifacts: ${artifacts}`)
