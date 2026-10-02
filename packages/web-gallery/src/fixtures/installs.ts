import type { HostInstall } from '@demicodes/web-ui/devices/installs'

/** How often a simulated install moves on, a tenth of an artifact at a time. */
const INSTALL_STEP_MS = 250

/** An artifact a specimen's Host installs: what its line shows, and whether it unpacks. */
export interface SimulatedArtifact {
  package: string
  name: string
  version: string
  total: number
  /** An archive unpacks once downloaded; a file does not. */
  archive?: boolean
}

/** What a Host installs before its first browser starts. */
export const BROWSER_ARTIFACTS: readonly SimulatedArtifact[] = [
  { package: 'demi.browser', name: 'program', version: '0.1.3', total: 41_943_040 },
  {
    package: 'demi.browser',
    name: 'Chrome for Testing',
    version: '153.0.8010.36',
    total: 195_711_476,
    archive: true,
  },
]

/** What the Cloud installs for a Claude Code request when it has no usable CLI. */
export const CLAUDE_CLI_ARTIFACTS: readonly SimulatedArtifact[] = [
  { package: 'demi.claude-code', name: 'Claude Code', version: '2.1.278', total: 125_829_120 },
]

/**
 * Plays `artifacts` the way a runner reports them (`native-runtime.md`
 * § Installation progress): one after another, each download a tenth at a
 * time, then an archive's unpacking. `show` receives each list, and an empty
 * one when the installs end or `signal` aborts them; the promise settles
 * then.
 */
export function playInstalls(
  artifacts: readonly SimulatedArtifact[],
  show: (installs: readonly HostInstall[]) => void,
  signal?: AbortSignal,
): Promise<void> {
  return new Promise((resolve) => {
    let index = 0
    let tenth = 0
    let timer: ReturnType<typeof setTimeout> | null = null
    const finish = () => {
      if (timer !== null) {
        clearTimeout(timer)
      }
      signal?.removeEventListener('abort', finish)
      show([])
      resolve()
    }
    const step = () => {
      const artifact = artifacts[index]
      if (!artifact) {
        finish()
        return
      }
      const archive = artifact.archive === true
      show([{
        package: artifact.package,
        name: artifact.name,
        version: artifact.version,
        phase: archive && tenth > 10 ? 'unpack' : 'download',
        done: Math.min(artifact.total, Math.round((artifact.total * tenth) / 10)),
        total: artifact.total,
      }])
      if (tenth >= (archive ? 14 : 10)) {
        index += 1
        tenth = 0
      } else {
        tenth += 1
      }
      timer = setTimeout(step, INSTALL_STEP_MS)
    }
    if (signal?.aborted) {
      finish()
      return
    }
    signal?.addEventListener('abort', finish, { once: true })
    step()
  })
}
