/**
 * Where the SDK finds its executables and its release, as esbuild finds its
 * binary: one package for each platform the runner is built for, each an
 * optional dependency of `@demicodes/sdk` that installs only on its own
 * platform, with `bin/demi-core`, `bin/demi-runner` and `release/files/`,
 * that platform's command programs.
 *
 * ```text
 * @demicodes/sdk                  release/commands/   every package's descriptor
 * @demicodes/sdk-darwin-arm64     bin/demi-core, bin/demi-runner, release/files/
 * @demicodes/sdk-linux-x64        ...
 * ```
 *
 * The process serves the command programs of every platform package
 * installed: a Host of another platform, such as a Linux container a Mac's
 * program starts, runs its commands once that platform's package is
 * installed too (`npm install --os=linux --cpu=x64
 * @demicodes/sdk-linux-x64`), which also holds the runner to run there.
 * `DEMI_SDK_PLATFORM` names a platform package's directory to use instead,
 * as for a development build.
 */
import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

/** Each platform's package, by Node's platform and architecture. */
const PLATFORMS: Record<string, string> = {
  'darwin-arm64': '@demicodes/sdk-darwin-arm64',
  'darwin-x64': '@demicodes/sdk-darwin-x64',
  'linux-arm64': '@demicodes/sdk-linux-arm64',
  'linux-x64': '@demicodes/sdk-linux-x64',
  'win32-arm64': '@demicodes/sdk-win32-arm64',
  'win32-x64': '@demicodes/sdk-win32-x64',
}

/** The executables of this platform, and the release's roots. */
export interface Binaries {
  core: string
  runner: string
  release: string[]
}

/** This platform's executables and the release; throws when its package is not installed. */
export function binaries(): Binaries {
  const require = createRequire(import.meta.url)
  const installed = (name: string): string | null => {
    try {
      return dirname(require.resolve(`${name}/package.json`))
    } catch {
      return null
    }
  }
  const platform = `${process.platform}-${process.arch}`
  const own = process.env.DEMI_SDK_PLATFORM ?? (PLATFORMS[platform] === undefined ? null : installed(PLATFORMS[platform]))
  if (own === null) {
    throw new Error(`No package of @demicodes/sdk for ${platform} is installed: install ${PLATFORMS[platform] ?? 'the SDK on a platform it is built for'}`)
  }
  const executable = (name: string) => join(own, 'bin', process.platform === 'win32' ? `${name}.exe` : name)
  const roots = [
    fileURLToPath(new URL('../release', import.meta.url)),
    join(own, 'release'),
    ...Object.values(PLATFORMS).flatMap((name) => {
      const directory = installed(name)
      return directory === null || directory === own ? [] : [join(directory, 'release')]
    }),
  ]
  return {
    core: executable('demi-core'),
    runner: executable('demi-runner'),
    release: roots.filter((root) => existsSync(root)),
  }
}
