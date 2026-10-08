import type { SentenceText, TitleText } from '../ui/ui-text'

/**
 * What a device's runner last reported about the machine and about itself
 * (`web-api.md` § Workspaces, devices, and attached hosts); each is null
 * before its runner first connected.
 */
export interface DeviceReport {
  /**
   * The operating system with its release, such as macOS 26.5, and the
   * architecture as Rust names it, such as x86_64 or aarch64.
   */
  os: { name: string; arch: string } | null
  /** The runner's release, the identity of the release it was installed from. */
  runnerVersion: string | null
}

/**
 * The chip family a person knows a machine's architecture by
 * (`direct-channel.md` § What the user sees): Apple silicon for a Mac on
 * ARM, ARM elsewhere, Intel for x86; null for another, which is not shown
 * rather than shown as a code.
 */
export function chipFamily(os: NonNullable<DeviceReport['os']>): TitleText | null {
  switch (os.arch) {
    case 'aarch64':
    case 'arm':
      return os.name.startsWith('macOS') ? 'Apple silicon' : 'ARM'
    case 'x86_64':
    case 'x86':
      return 'Intel'
    default:
      return null
  }
}

/** The system by its name and release with its chip family, such as "macOS 26.5 · Apple silicon"; null before the runner reported one. */
export function systemName(report: DeviceReport): string | null {
  if (!report.os) {
    return null
  }
  const chip = chipFamily(report.os)
  return chip ? `${report.os.name} · ${chip}` : report.os.name
}

/**
 * The device's runner against the release the server's runners follow:
 * `current` when it runs it, `outdated` when it runs another, which it
 * replaces as it next connects, `development` on a server without runner
 * releases, as in development; null before the runner reported one.
 */
export type RunnerState = 'current' | 'outdated' | 'development'

export function runnerState(runnerVersion: string | null, serverRelease: string | null): RunnerState | null {
  if (serverRelease === null) {
    return 'development'
  }
  if (runnerVersion === null) {
    return null
  }
  return runnerVersion === serverRelease ? 'current' : 'outdated'
}

export const RUNNER_STATE_LABEL: Record<RunnerState, SentenceText> = {
  current: 'Up to date',
  outdated: 'Update available',
  development: 'Development build',
}
