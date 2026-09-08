import { z } from 'zod'
// Machine-local runner state (`runner.md` § Process shape and local state):
//
//   <stateDir>/runner.json    backend URL, device id
//   <stateDir>/runner-token   device token (0600)
import type { HostFileSystem } from '@demicodes/shell'
import { decodeUtf8, encodeUtf8, isFileNotFoundError } from '@demicodes/utils'

const runnerConfigSchema = z.object({ backendUrl: z.string().url(), deviceId: z.string().optional() }).strict()
export const activeRunnerSchema = z.object({ endpoint: z.string().min(1), secret: z.string().regex(/^[a-f0-9]{32}$/), release: z.string().min(1) }).strict()
export type RunnerConfig = z.infer<typeof runnerConfigSchema>

export class RunnerState {
  constructor(
    private readonly fs: HostFileSystem,
    readonly dir: string,
  ) {}

  get configPath(): string {
    return `${this.dir}/runner.json`
  }

  get tokenPath(): string {
    return `${this.dir}/runner-token`
  }

  get activePath(): string { return `${this.dir}/active.json` }

  get commandsDir(): string {
    return `${this.dir}/commands`
  }

  get outputDir(): string {
    return `${this.dir}/output`
  }

  async readConfig(): Promise<RunnerConfig | null> {
    try {
      return runnerConfigSchema.parse(JSON.parse(decodeUtf8(await this.fs.readFile(this.configPath))))
    } catch (error) {
      if (isFileNotFoundError(error)) return null
      throw error
    }
  }

  async writeConfig(config: RunnerConfig): Promise<void> {
    await this.fs.writeFile(this.configPath, encodeUtf8(`${JSON.stringify(config, null, 2)}\n`), { createParents: true })
  }

  async readToken(): Promise<string | null> {
    try {
      const token = decodeUtf8(await this.fs.readFile(this.tokenPath)).trim()
      return token.length > 0 ? token : null
    } catch (error) {
      if (isFileNotFoundError(error)) return null
      throw error
    }
  }

  async writeToken(token: string): Promise<void> {
    await this.fs.writeFile(this.tokenPath, encodeUtf8(`${token}\n`), { createParents: true })
    await this.fs.chmod(this.tokenPath, 0o600)
  }
}
