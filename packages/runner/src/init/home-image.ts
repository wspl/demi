// The home image as the guest sees it (`managed-hosts.md` § Home
// persistence): a block device under the home mount. The runner flushes it
// before a hibernate, reports whether anything wrote to it since boot —
// the block layer's own count of sectors written, from `/proc/diskstats`,
// against the baseline taken right after the mount — asks for growth when
// the filesystem nears its cap, and grows the filesystem into the image the
// backend enlarged.
import { decodeUtf8 } from '@demicodes/utils'

/** What the runner mode needs of the home; the default over a plain directory syncs and knows nothing else. */
export interface HomeImage {
  /** Flushes the home to disk; `untouched` when nothing wrote since boot. */
  sync(): Promise<{ untouched: boolean }>
  /** The total size the home should grow to, or null while there is room. */
  wanted(): Promise<number | null>
  /** The backing image is now `bytes` large: grow the filesystem into it. */
  grown(bytes: number): Promise<void>
}

/** The sectors written to `device` (its basename, `vdb`) as `/proc/diskstats` counts them; null when the device is absent. */
export function sectorsWritten(diskstats: string, device: string): number | null {
  for (const line of diskstats.split('\n')) {
    const fields = line.trim().split(/\s+/)
    // major minor name reads-completed reads-merged sectors-read ms-reading writes-completed writes-merged sectors-written …
    if (fields[2] === device && fields.length >= 10) return Number(fields[9])
  }
  return null
}

export interface HomeUsage {
  totalBytes: number
  availableBytes: number
}

/** `df -B1 -P <mount>` as numbers: the filesystem's size and what is left. */
export function parseDf(output: string): HomeUsage | null {
  const lines = output.trim().split('\n')
  const fields = lines[lines.length - 1]?.trim().split(/\s+/) ?? []
  if (lines.length < 2 || fields.length < 6) return null
  const totalBytes = Number(fields[1])
  const availableBytes = Number(fields[3])
  if (!Number.isFinite(totalBytes) || !Number.isFinite(availableBytes)) return null
  return { totalBytes, availableBytes }
}

export interface GrowthPolicy {
  /** Ask for growth when less than this fraction of the filesystem is free … */
  reserveFraction: number
  /** … or less than this many bytes, for a filesystem large enough that a tenth is too little … */
  reserveBytes: number
  /** … but never more than this fraction, so a small filesystem is not asked to grow at once. */
  reserveCapFraction: number
  /** The size asked for is the current one times this. */
  factor: number
}

export const DEFAULT_GROWTH_POLICY: GrowthPolicy = { reserveFraction: 0.1, reserveBytes: 256 * 1024 * 1024, reserveCapFraction: 0.25, factor: 2 }

/** The reserve: a tenth of the filesystem, raised toward 256 MB but never past a quarter. */
export function reserveBytes(totalBytes: number, policy: GrowthPolicy = DEFAULT_GROWTH_POLICY): number {
  return Math.max(totalBytes * policy.reserveFraction, Math.min(policy.reserveBytes, totalBytes * policy.reserveCapFraction))
}

/** The size to ask for, or null while the reserve holds. */
export function growthWanted(usage: HomeUsage, policy: GrowthPolicy = DEFAULT_GROWTH_POLICY): number | null {
  if (usage.availableBytes >= reserveBytes(usage.totalBytes, policy)) return null
  return Math.ceil(usage.totalBytes * policy.factor)
}

export interface HomeImageIO {
  run(command: string, args: string[]): Promise<{ code: number | null; stdout: Uint8Array }>
  readFile(path: string): Promise<Uint8Array>
}

/** The image behind `device` mounted at `mount`, for PID 1. `baseline()` right after the mount, before anything runs. */
export class BlockHomeImage implements HomeImage {
  private written: number | null = null

  constructor(
    private readonly io: HomeImageIO,
    private readonly device: string,
    private readonly mount: string,
    private readonly policy: GrowthPolicy = DEFAULT_GROWTH_POLICY,
  ) {}

  async baseline(): Promise<void> {
    this.written = await this.sectors()
  }

  async sync(): Promise<{ untouched: boolean }> {
    await this.io.run('sync', ['-f', this.mount])
    const now = await this.sectors()
    return { untouched: this.written !== null && now !== null && now === this.written }
  }

  async wanted(): Promise<number | null> {
    const df = await this.io.run('df', ['-B1', '-P', this.mount])
    const usage = df.code === 0 ? parseDf(decodeUtf8(df.stdout)) : null
    return usage ? growthWanted(usage, this.policy) : null
  }

  async grown(bytes: number): Promise<void> {
    const result = await this.io.run('resize2fs', [this.device])
    if (result.code !== 0) throw new Error(`resize2fs ${this.device} exited ${result.code ?? 'by signal'} after growth to ${bytes} bytes`)
  }

  private async sectors(): Promise<number | null> {
    try {
      return sectorsWritten(decodeUtf8(await this.io.readFile('/proc/diskstats')), this.device.replace(/^\/dev\//, ''))
    } catch {
      return null
    }
  }
}

/** A home that is a plain directory (a user host, the test fake): `sync` flushes everything and nothing else applies. */
export class DirectoryHome implements HomeImage {
  constructor(private readonly io: Pick<HomeImageIO, 'run'>) {}

  async sync(): Promise<{ untouched: boolean }> {
    await this.io.run('sync', [])
    return { untouched: false }
  }

  async wanted(): Promise<number | null> {
    return null
  }

  async grown(): Promise<void> {}
}
