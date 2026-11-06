/**
 * A program's Demi: the process its cores run in, which `Demi.start`
 * starts with the program's plugins, and the cores it opens.
 */
import { resolve } from 'node:path'
import { openedCoreSchema } from '@demicodes/protocol'
import { binaries } from './binaries'
import { Core } from './core'
import type { Plugin } from './plugin'
import { Process } from './process'
import { PROCESS_ORIGIN, refusal } from './routes'

/** What Demi starts with. */
export interface DemiOptions {
  /** The program's plugins, which its cores' agents use besides Demi's own. */
  plugins?: readonly Plugin[]
  /**
   * Where the program's server forwards the process's routes, such as
   * `https://bot.example.com/demi`; without it, the cores offer neither a
   * plugin's sign-in nor a schedule's URL.
   */
  publicUrl?: string
}

export class Demi {
  private readonly cores = new Map<string, Core>()

  private constructor(
    private readonly process: Process,
    private readonly plugins: readonly Plugin[],
    private readonly runner: string | null,
  ) {}

  /** Starts the process with the program's plugins. */
  static async start(options: DemiOptions = {}): Promise<Demi> {
    const found = binaries()
    const process = Process.start({
      executable: found.core,
      release: found.release,
      ...(options.publicUrl === undefined ? {} : { publicUrl: options.publicUrl }),
    })
    const plugins = options.plugins ?? []
    const demi = new Demi(process, plugins, found.runner)
    await demi.call('PUT', '/plugins', { plugins: plugins.map((plugin) => plugin.manifest) })
    return demi
  }

  /**
   * Opens the core in `dir`, creating it when `dir` is absent or empty, and
   * reads its product state, which it follows from then on.
   */
  async open(dir: string): Promise<Core> {
    const path = resolve(dir)
    const response = await this.call('POST', '/cores', { dir: path })
    const { id } = openedCoreSchema.parse(await response.json())
    const core = new Core(
      { process: this.process, id, dir: path, plugins: this.plugins, runner: this.runner },
      async () => {
        this.cores.delete(id)
        await (await this.call('DELETE', `/cores/${encodeURIComponent(id)}`)).body?.cancel()
      },
    )
    this.cores.set(id, core)
    await core.follow()
    return core
  }

  /**
   * Answers a call of a schedule's URL, `/hooks/{id}` below `base`, as the
   * program's server forwards it.
   */
  fetch(request: Request, base = ''): Promise<Response> {
    const url = new URL(request.url)
    const path = url.pathname.startsWith(base) ? url.pathname.slice(base.length) : url.pathname
    return this.process.fetch(new Request(PROCESS_ORIGIN + path + url.search, request))
  }

  /** Closes every core and ends the process. */
  async close(): Promise<void> {
    await Promise.all([...this.cores.values()].map((core) => core.close()))
    await this.process.close()
  }

  private async call(method: string, path: string, body?: unknown): Promise<Response> {
    const response = await this.process.fetch(new Request(PROCESS_ORIGIN + path, {
      method,
      ...(body === undefined ? {} : { body: JSON.stringify(body), headers: { 'content-type': 'application/json' } }),
    }))
    if (!response.ok) {
      throw await refusal(response, `${method} ${path}`)
    }
    return response
  }
}
