import { Hono } from 'hono'
import { join } from 'node:path'
import {
  releaseDigest,
  releaseTarget,
  runnerReleaseSchema,
} from '@demicodes/runner-protocol/release'
import { shellInstaller } from '../runner/installer'

export type { RunnerRelease } from '@demicodes/runner-protocol/release'

export interface RunnerInstallationOptions {
  directory: string
  backendUrl?: string
}

/**
 * Public downloads contain no credential; device access still requires pairing.
 */
export function runnerInstallRoutes(options?: RunnerInstallationOptions): Hono {
  const app = new Hono()

  app.get('/install.sh', async context => {
    if (!options) {
      return context.text(
        'Runner releases are not configured on this backend.\n',
        503
      )
    }
    const manifest = await Bun.file(join(options.directory, 'manifest.json')).json()
    const release = runnerReleaseSchema.parse(manifest)
    const backendUrl = options.backendUrl ?? new URL(context.req.url).origin
    return context.body(shellInstaller(backendUrl, release), 200, {
      'content-type': 'text/x-shellscript; charset=utf-8',
      'cache-control': 'no-store',
    })
  })

  app.get('/install.ps1', context => {
    return context.text('A Windows runner release is not available yet.\n', 503)
  })

  app.get('/runner-artifacts/:release/:target/:file', async context => {
    if (!options) {
      return context.notFound()
    }
    const requestedRelease = releaseDigest.safeParse(
      context.req.param('release')
    )
    const platform = releaseTarget.safeParse(context.req.param('target'))
    const name = context.req.param('file')
    if (!requestedRelease.success ||
      !platform.success ||
      !['demi', 'demi-runner'].includes(name)) {
      return context.notFound()
    }

    const releaseDirectory = join(options.directory, requestedRelease.data)
    const manifest = Bun.file(join(releaseDirectory, 'manifest.json'))
    if (!await manifest.exists()) {
      return context.notFound()
    }
    const release = runnerReleaseSchema.parse(await manifest.json())
    if (release.release !== requestedRelease.data ||
      !release.targets[platform.data]) {
      return context.notFound()
    }

    const file = Bun.file(join(releaseDirectory, platform.data, name))
    if (!await file.exists()) {
      return context.notFound()
    }
    return new Response(file, {
      headers: {
        'content-type': 'application/octet-stream',
        'cache-control': 'public, max-age=31536000, immutable',
      },
    })
  })

  return app
}
