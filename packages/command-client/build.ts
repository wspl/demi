import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs'
import { join, resolve } from 'node:path'
import contract from '../runner-protocol/src/local-contract.json'

const root = resolve(import.meta.dir, '../..')
const built = new Map<string, string>()

function run(args: string[], env: NodeJS.ProcessEnv): void {
  const result = Bun.spawnSync(
    args,
    { env, stdout: 'inherit', stderr: 'inherit' }
  )
  if (!result.success) {
    throw new Error(`native client build failed: ${args.join(' ')}`)
  }
}

function writeContractHeader(directory: string): void {
  mkdirSync(join(directory, 'generated'), { recursive: true })
  const lines = [
    '#ifndef DEMI_CONTRACT_H',
    '#define DEMI_CONTRACT_H',
    `#define DEMI_VERSION ${contract.version}`,
    `#define DEMI_MAX_FRAME ${contract.maxFrame}`,
    `#define DEMI_CHUNK_SIZE ${contract.chunkSize}`,
    `#define DEMI_ENDPOINT_ENV "${contract.endpointEnv}"`,
    `#define DEMI_CONTEXT_ENV "${contract.contextEnv}"`,
    `#define DEMI_STDIN_FD_ENV "${contract.stdinFdEnv}"`,
  ]
  for (const [name, value] of Object.entries(contract.frames)) {
    const constant = name.replace(
      /[A-Z]/g,
      character => `_${character}`
    ).toUpperCase()
    lines.push(`#define DEMI_${constant} ${value}`)
  }
  const header = `${lines.join('\n')}\n#endif\n`
  const path = join(directory, 'generated/contract.h')
  if (!existsSync(path) || readFileSync(path, 'utf8') !== header) {
    writeFileSync(path, header)
  }
}

function targetConfiguration(target?: string) {
  const flags = [
    '-DCMAKE_BUILD_TYPE=MinSizeRel',
    '-DCMAKE_C_FLAGS=-ffunction-sections -fdata-sections',
  ]
  let env = process.env

  if (target?.endsWith('-linux-musl')) {
    if (!/^(aarch64|x86_64)-linux-musl$/.test(target)) {
      throw new Error(`unsupported client target: ${target}`)
    }
    env = {
      ...env,
      DEMI_ZIG_TARGET: target,
      DEMI_ZIG_ARCH: target.split('-')[0]
    }
    flags.push(
      `-DCMAKE_TOOLCHAIN_FILE=${join(root, 'packages/runner/runtime/toolchain/linux-musl.cmake')}`
    )
  } else if (target?.endsWith('-macos')) {
    if (process.platform !== 'darwin'
      || !/^(arm64|x86_64)-macos$/.test(target)) {
      throw new Error(`unsupported client target: ${target}`)
    }
    flags.push(`-DCMAKE_OSX_ARCHITECTURES=${target.split('-')[0]}`)
  } else if (target === 'x86_64-windows-gnu') {
    env = { ...env, DEMI_ZIG_TARGET: target, DEMI_ZIG_ARCH: 'AMD64' }
    flags.push(
      `-DCMAKE_TOOLCHAIN_FILE=${join(import.meta.dir, 'toolchain/windows-gnu.cmake')}`
    )
  } else if (target) {
    throw new Error(`unsupported client target: ${target}`)
  }

  return { flags, env }
}

function linkWindowsClient(
  directory: string,
  binary: string,
  env: NodeJS.ProcessEnv
): void {
  // Direct linking avoids Zig/CMake whole-archive CRT incompatibility.
  run([
    process.env.ZIG ?? 'zig', 'cc',
    '-target', 'x86_64-windows-gnu', '-Os', '-s',
    `-I${join(root, 'vendor/txiki.js/deps/libuv/include')}`,
    `-I${join(directory, 'generated')}`,
    join(import.meta.dir, 'src/client.c'),
    join(import.meta.dir, 'src/metadata.c'),
    join(directory, 'libuv/libuv.a'),
    '-lpsapi', '-luser32', '-ladvapi32', '-liphlpapi', '-luserenv',
    '-lws2_32', '-ldbghelp', '-lole32', '-lshell32',
    '-o', binary,
  ], env)
}

export function commandClientBinary(target?: string): string {
  const key = target ?? 'host'
  const cached = built.get(key)
  if (cached) {
    return cached
  }

  const directory = join(root, '.cache/command-client', key)
  writeContractHeader(directory)
  const cmake = process.env.CMAKE ?? 'cmake'
  const { flags, env } = targetConfiguration(target)
  run([cmake, '-S', import.meta.dir, '-B', directory, ...flags], env)

  const crossCompileWindows = target === 'x86_64-windows-gnu'
  const windowsBinary = crossCompileWindows
    || (process.platform === 'win32' && !target)
  const binary = join(directory, windowsBinary ? 'demi.exe' : 'demi')
  const buildTarget = crossCompileWindows ? 'uv_a' : 'demi'
  run([cmake, '--build', directory, '--target', buildTarget, '-j', '8'], env)
  if (crossCompileWindows) {
    linkWindowsClient(directory, binary, env)
  }

  built.set(key, binary)
  return binary
}

if (import.meta.main) {
  console.log(commandClientBinary(process.argv[2]))
}
