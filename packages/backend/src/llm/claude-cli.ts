import type { CommandContext, NativePackage, ArtifactResolver } from '@demicodes/command-protocol'
import type { RemoteHost } from '@demicodes/host-remote'
import { errorMessage } from '@demicodes/utils'
import { z } from 'zod'

/** The vendor's official distribution (`claude-cli.md` § Which version). */
const RELEASES_URL = 'https://downloads.claude.ai/claude-code-releases'
const VERSION = /^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$/
/** How long the newest version is believed before the pointer is read again. */
const LATEST_TTL_MS = 6 * 60 * 60_000

const manifestSchema = z.looseObject({
  version: z.string().regex(VERSION),
  platforms: z.record(z.string().min(1), z.looseObject({
    binary: z.string().min(1),
    checksum: z.string().regex(/^[a-f0-9]{64}$/),
    size: z.number().int().positive(),
  })),
})

/** What `claude.ensure` installs: one version, and each platform's verified download. */
export interface ClaudeRelease {
  version: string
  platforms: Record<string, { url: string; size: number; sha256: string }>
}

const ensuredSchema = z.discriminatedUnion('ok', [
  z.strictObject({ ok: z.literal(true), version: z.string(), path: z.string().min(1) }),
  z.strictObject({ ok: z.literal(false), code: z.string(), message: z.string() }),
])
const statusSchema = z.discriminatedUnion('ok', [
  z.strictObject({
    ok: z.literal(true),
    platform: z.string(),
    installed: z.array(z.strictObject({ version: z.string(), path: z.string().min(1) })),
  }),
  z.strictObject({ ok: z.literal(false), code: z.string(), message: z.string() }),
])

/** Why a machine has no usable CLI, with the version that was wanted. */
export class ClaudeCliError extends Error {
  constructor(
    readonly code: string,
    readonly version: string | null,
    message: string
  ) {
    super(version
      ? `Claude Code ${version} could not be installed: ${message}`
      : `Claude Code could not be installed: ${message}`)
    this.name = 'ClaudeCliError'
  }
}

/** The vendor's releases, read from its distribution and believed for a while. */
export class ClaudeReleases {
  private newest: { release: ClaudeRelease; readAt: number } | null = null
  private reading: Promise<ClaudeRelease> | null = null
  private readonly held = new Map<string, ClaudeRelease>()

  constructor(private readonly options: {
    fetch?: typeof fetch
    now?: () => number
  } = {}) {}

  /** The vendor's newest release; `refresh` reads the pointer again at once. */
  async latest(refresh = false): Promise<ClaudeRelease> {
    const now = this.options.now?.() ?? Date.now()
    if (!refresh && this.newest && now - this.newest.readAt < LATEST_TTL_MS)
      return this.newest.release
    this.reading ??= (async () => {
      try {
        const version = (await this.text(`${RELEASES_URL}/latest`)).trim()
        if (!VERSION.test(version))
          throw new Error('the distribution named no version')
        const release = await this.release(version)
        this.newest = { release, readAt: this.options.now?.() ?? Date.now() }
        return release
      } finally {
        this.reading = null
      }
    })()
    return this.reading
  }

  /** One version's release record. A release never changes once published. */
  async release(version: string): Promise<ClaudeRelease> {
    if (!VERSION.test(version))
      throw new ClaudeCliError('invalid_release', null, `"${version}" is not a version`)
    const known = this.held.get(version)
    if (known)
      return known
    let manifest: z.infer<typeof manifestSchema>
    try {
      manifest = manifestSchema.parse(
        JSON.parse(await this.text(`${RELEASES_URL}/${version}/manifest.json`))
      )
    } catch (error) {
      throw new ClaudeCliError(
        'release_unreadable',
        version,
        `its release could not be read (${errorMessage(error)})`
      )
    }
    const release: ClaudeRelease = {
      version: manifest.version,
      platforms: Object.fromEntries(Object.entries(manifest.platforms).map(
        ([platform, entry]) => [platform, {
          url: `${RELEASES_URL}/${manifest.version}/${platform}/${entry.binary}`,
          size: entry.size,
          sha256: entry.checksum,
        }]
      )),
    }
    this.held.set(version, release)
    return release
  }

  private async text(url: string): Promise<string> {
    let response: Response
    try {
      response = await (this.options.fetch ?? fetch)(url, {
        redirect: 'error',
        signal: AbortSignal.timeout(15_000),
      })
    } catch (error) {
      throw new ClaudeCliError(
        'release_unreadable',
        null,
        `the distribution did not answer (${errorMessage(error)})`
      )
    }
    if (!response.ok)
      throw new ClaudeCliError(
        'release_unreadable',
        null,
        `the distribution answered ${response.status}`
      )
    return response.text()
  }
}

/** A machine the CLI is wanted on, and the command context the work runs under. */
export interface ClaudeCliTarget {
  host: RemoteHost
  /** Tells machines apart for the installs this backend has under way. */
  deviceId: string
  cwd: string
  context: CommandContext
}

/**
 * Demi's own Claude Code CLI on the machines that run it (`claude-cli.md`):
 * asks the `demi.claude` package for a verified executable of the wanted
 * version and answers its path.
 */
export class ClaudeCli {
  private readonly upgrading = new Map<string, Promise<void>>()

  /** The vendor's releases this installer asks for. */
  get releases(): ClaudeReleases {
    return this.options.releases
  }

  constructor(private readonly options: {
    releases: ClaudeReleases
    package: NativePackage
    resolveArtifact: ArtifactResolver
  }) {}

  /**
   * The executable to start on `target`. A machine with a usable version
   * answers with it at once and installs the newest beside it; only a machine
   * with none waits for an install. `held` names the version an entry is held
   * at, which is then the only one wanted.
   */
  async executable(
    target: ClaudeCliTarget,
    options: { held?: string; signal?: AbortSignal } = {}
  ): Promise<string> {
    const wanted = options.held
      ? await this.options.releases.release(options.held)
      : await this.options.releases.latest()
    const installed = await this.installed(target, options.signal)
    const usable = installed.find(entry => entry.version === wanted.version)
      ?? (options.held ? undefined : installed[0])
    if (!usable)
      return this.ensure(target, wanted, options.signal)
    if (usable.version !== wanted.version)
      this.upgrade(target, wanted)
    return usable.path
  }

  /**
   * Where a CLI process runs on `target`: the executable, and directories of
   * Demi's under the machine's home, made first. The CLI keeps nothing there
   * that a conversation owns, so every process of a machine shares them.
   */
  async place(
    target: ClaudeCliTarget,
    options: { held?: string; signal?: AbortSignal } = {}
  ): Promise<{ command: string; cwd: string; configDir: string }> {
    const command = await this.executable(target, options)
    const cwd = `${target.cwd}/.demi/claude/run`
    const configDir = `${target.cwd}/.demi/claude/config`
    await target.host.fs.mkdir(cwd, { recursive: true })
    await target.host.fs.mkdir(configDir, { recursive: true })
    return { command, cwd, configDir }
  }

  /** Installs `release` on `target` and answers the executable's path. */
  async ensure(
    target: ClaudeCliTarget,
    release: ClaudeRelease,
    signal?: AbortSignal
  ): Promise<string> {
    const answer = ensuredSchema.parse(
      await this.call(target, 'claude.ensure', release, signal)
    )
    if (!answer.ok)
      throw new ClaudeCliError(answer.code, release.version, answer.message)
    return answer.path
  }

  /** The versions `target` has, newest first. */
  async installed(
    target: ClaudeCliTarget,
    signal?: AbortSignal
  ): Promise<Array<{ version: string; path: string }>> {
    const answer = statusSchema.parse(
      await this.call(target, 'claude.status', null, signal)
    )
    if (!answer.ok)
      throw new ClaudeCliError(answer.code, null, answer.message)
    return answer.installed
  }

  /** One install of the newer version per machine, whoever noticed it first. */
  private upgrade(target: ClaudeCliTarget, release: ClaudeRelease): void {
    const key = `${target.deviceId}\0${release.version}`
    if (this.upgrading.has(key))
      return
    // Nothing waits for it: a failed update leaves the version in use, and the
    // next request notices the newer one again.
    this.upgrading.set(key, this.ensure(target, release)
      .then(() => undefined, () => undefined)
      .finally(() => this.upgrading.delete(key)))
  }

  private async call(
    target: ClaudeCliTarget,
    operation: 'claude.ensure' | 'claude.status',
    input: unknown,
    signal?: AbortSignal
  ): Promise<unknown> {
    let bytes: Uint8Array
    try {
      bytes = await target.host.services.call({
        context: target.context,
        package: this.options.package,
        operation,
        cwd: target.cwd,
        input: new TextEncoder().encode(input === null ? '' : JSON.stringify(input)),
        resolveArtifact: this.options.resolveArtifact,
        maxBytes: 1024 * 1024,
        signal,
      })
    } catch (error) {
      if (signal?.aborted)
        throw error
      throw new ClaudeCliError('service_failed', null, errorMessage(error))
    }
    const text = new TextDecoder().decode(bytes).trim()
    if (!text)
      throw new ClaudeCliError('service_failed', null, 'the installer gave no answer')
    return JSON.parse(text)
  }
}
