import { copyFileSync, mkdirSync } from 'node:fs'
import { resolve, join } from 'node:path'

const root = resolve(import.meta.dir, '../../..')
const source = join(root, 'vendor/txiki.js')
const cmake = process.env.CMAKE ?? 'cmake'
const hostDir = join(root, '.cache/txiki/host')
const flags = [
  '-DCMAKE_BUILD_TYPE=MinSizeRel',
  '-DBUILD_WITH_FFI=OFF',
  '-DBUILD_WITH_WASM=OFF',
  '-DBUILD_WITH_SQLITE=OFF',
  '-DBUILD_WITH_MIMALLOC=OFF',
  '-DBUILD_WITH_GC_SECTIONS=ON',
]
function run(args: string[], env = process.env): void {
  const result = Bun.spawnSync(
    args,
    { env, stdout: 'inherit', stderr: 'inherit' }
  )
  if (!result.success)
    throw new Error(`build failed: ${args.join(' ')}`)
}
let hostBuilt = false
export function runtimeBinary(): string {
  if (!hostBuilt) {
    run([
      cmake,
      '-S',
      source,
      '-B',
      hostDir,
      ...flags,
      '-DBUILD_WITH_LTO=ON',
      '-DBUILD_WITH_STRIP=ON'
    ])
    run([cmake, '--build', hostDir, '--target', 'tjs-cli', 'tjsc', '-j', '8'])
    hostBuilt = true
  }
  return join(hostDir, 'tjs')
}

/**
 * Link with the pinned runtime and host bytecode compiler; the result is a
 * native executable.
 */
export function packRuntime(
  entry: string,
  output: string,
  target?: string
): void {
  runtimeBinary()
  const buildDir = target ? join(root, '.cache/txiki', target) : hostDir
  const options = [
    ...flags,
    `-DTJS_APP_ENTRY=${resolve(entry)}`,
    `-DTJS_BYTECODE_COMPILER=${join(hostDir, 'tjsc')}`
  ]
  let env = process.env
  if (target?.endsWith('-macos')) {
    if (process.platform !== 'darwin' || !/^(arm64|x86_64)-macos$/.test(target))
      throw new Error(`unsupported target: ${target}`)
    options.push(
      `-DCMAKE_OSX_ARCHITECTURES=${target.split('-')[0]}`,
      '-DBUILD_WITH_LTO=ON',
      '-DBUILD_WITH_STRIP=ON'
    )
  } else if (target) {
    if (!/^(aarch64|x86_64)-linux-musl$/.test(target))
      throw new Error(`unsupported target: ${target}`)
    env = {
      ...process.env,
      DEMI_ZIG_TARGET: target,
      DEMI_ZIG_ARCH: target.split('-')[0]
    }
    const toolchain = join(import.meta.dir, 'toolchain/linux-musl.cmake')
    options.push(
      `-DCMAKE_TOOLCHAIN_FILE=${toolchain}`,
      '-DBUILD_WITH_LTO=OFF',
      '-DBUILD_WITH_STRIP=OFF'
    )
  } else {
    options.push('-DBUILD_WITH_LTO=ON', '-DBUILD_WITH_STRIP=ON')
  }
  run([cmake, '-S', source, '-B', buildDir, ...options], env)
  run([cmake, '--build', buildDir, '--target', 'tjs-app', '-j', '8'], env)
  mkdirSync(resolve(output, '..'), { recursive: true })
  copyFileSync(join(buildDir, 'tjs-app'), output)
}

if (import.meta.main) {
  const [entry, output, target] = process.argv.slice(2)
  if (!entry || !output)
    throw new Error('Usage: bun runtime/build.ts entry.mjs output [target]')
  packRuntime(entry, output, target)
}
