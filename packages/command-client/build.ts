import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs'
import { join, resolve } from 'node:path'
import contract from '../runner-protocol/src/local-contract.json'

const root = resolve(import.meta.dir, '../..')
const built = new Map<string, string>()
function run(args: string[], env: NodeJS.ProcessEnv): void {
  const result = Bun.spawnSync(args, { env, stdout: 'inherit', stderr: 'inherit' })
  if (!result.success) throw new Error(`native client build failed: ${args.join(' ')}`)
}
export function commandClientBinary(target?: string): string {
  const key = target ?? 'host'
  if (built.has(key)) return built.get(key)!
  const dir = join(root, '.cache/command-client', key)
  mkdirSync(join(dir, 'generated'), { recursive: true })
  const lines = ['#ifndef DEMI_CONTRACT_H', '#define DEMI_CONTRACT_H', `#define DEMI_VERSION ${contract.version}`, `#define DEMI_MAX_FRAME ${contract.maxFrame}`, `#define DEMI_CHUNK_SIZE ${contract.chunkSize}`, `#define DEMI_ENDPOINT_ENV "${contract.endpointEnv}"`, `#define DEMI_CONTEXT_ENV "${contract.contextEnv}"`, `#define DEMI_STDIN_FD_ENV "${contract.stdinFdEnv}"`]
  for (const [name, value] of Object.entries(contract.frames)) lines.push(`#define DEMI_${name.replace(/[A-Z]/g, c => `_${c}`).toUpperCase()} ${value}`)
  const header = `${lines.join('\n')}\n#endif\n`
  const path = join(dir, 'generated/contract.h')
  if (!existsSync(path) || readFileSync(path, 'utf8') !== header) writeFileSync(path, header)
  const cmake = process.env.CMAKE ?? 'cmake'
  let env = process.env
  const flags = ['-DCMAKE_BUILD_TYPE=MinSizeRel', '-DCMAKE_C_FLAGS=-ffunction-sections -fdata-sections']
  if (target?.endsWith('-linux-musl')) {
    if (!/^(aarch64|x86_64)-linux-musl$/.test(target)) throw new Error(`unsupported client target: ${target}`)
    env = { ...env, DEMI_ZIG_TARGET: target, DEMI_ZIG_ARCH: target.split('-')[0] }
    flags.push(`-DCMAKE_TOOLCHAIN_FILE=${join(root, 'packages/runner/runtime/toolchain/linux-musl.cmake')}`)
  } else if (target?.endsWith('-macos')) {
    if (process.platform !== 'darwin' || !/^(arm64|x86_64)-macos$/.test(target)) throw new Error(`unsupported client target: ${target}`)
    flags.push(`-DCMAKE_OSX_ARCHITECTURES=${target.split('-')[0]}`)
  } else if (target === 'x86_64-windows-gnu') {
    env = { ...env, DEMI_ZIG_TARGET: target, DEMI_ZIG_ARCH: 'AMD64' }
    flags.push(`-DCMAKE_TOOLCHAIN_FILE=${join(import.meta.dir, 'toolchain/windows-gnu.cmake')}`)
  } else if (target) throw new Error(`unsupported client target: ${target}`)
  run([cmake, '-S', import.meta.dir, '-B', dir, ...flags], env)
  const windows = target === 'x86_64-windows-gnu'
  const binary = join(dir, windows || (process.platform === 'win32' && !target) ? 'demi.exe' : 'demi')
  run([cmake, '--build', dir, '--target', windows ? 'uv_a' : 'demi', '-j', '8'], env)
  if (windows) {
    // Direct linking avoids Zig/CMake whole-archive CRT incompatibility.
    run([process.env.ZIG ?? 'zig', 'cc', '-target', target, '-Os', '-s', `-I${join(root, 'vendor/txiki.js/deps/libuv/include')}`, `-I${join(dir, 'generated')}`, join(import.meta.dir, 'src/client.c'), join(import.meta.dir, 'src/metadata.c'), join(dir, 'libuv/libuv.a'), '-lpsapi', '-luser32', '-ladvapi32', '-liphlpapi', '-luserenv', '-lws2_32', '-ldbghelp', '-lole32', '-lshell32', '-o', binary], env)
  }
  built.set(key, binary)
  return binary
}
if (import.meta.main) console.log(commandClientBinary(process.argv[2]))
