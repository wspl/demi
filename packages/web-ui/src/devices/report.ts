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
  /** The runner's release, such as 0.1.16. */
  runnerVersion: string | null
}

/**
 * The name people know an architecture by where Rust's differs: 64-bit ARM
 * is arm64 to macOS and Windows, aarch64 only to Linux. Any other reads as
 * its runner reported it.
 */
const ARCH_LABEL: Readonly<Record<string, string>> = {
  aarch64: 'arm64',
}

/**
 * The line under a device's name that says what it runs, such as
 * "macOS 26.5 · arm64 · Runner 0.1.16"; null while its runner has reported
 * nothing.
 */
export function deviceReportLine(report: DeviceReport): string | null {
  const parts: string[] = []
  if (report.os) {
    parts.push(report.os.name, ARCH_LABEL[report.os.arch] ?? report.os.arch)
  }
  if (report.runnerVersion) {
    parts.push(`Runner ${report.runnerVersion}`)
  }
  return parts.length ? parts.join(' · ') : null
}
