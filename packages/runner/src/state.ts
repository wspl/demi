import { chmod, mkdir, readFile, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'

/**
 * Machine-local runner state:
 *
 *   <stateDir>/runner.json    backend URL, device id
 *   <stateDir>/runner-token   device token (0600)
 */

export interface RunnerConfig {
  backendUrl: string
  deviceId?: string
}

export class RunnerState {
  constructor(private readonly stateDir: string = join(homedir(), '.demi')) {}

  private get configPath(): string {
    return join(this.stateDir, 'runner.json')
  }

  private get tokenPath(): string {
    return join(this.stateDir, 'runner-token')
  }

  async readConfig(): Promise<RunnerConfig | null> {
    try {
      return parsePortableJson<RunnerConfig>(await readFile(this.configPath, 'utf8'))
    } catch {
      return null
    }
  }

  async writeConfig(config: RunnerConfig): Promise<void> {
    await mkdir(this.stateDir, { recursive: true })
    await writeFile(this.configPath, stringifyPortableJson(config, 2))
  }

  async readToken(): Promise<string | null> {
    try {
      const token = (await readFile(this.tokenPath, 'utf8')).trim()
      return token.length > 0 ? token : null
    } catch {
      return null
    }
  }

  async writeToken(token: string): Promise<void> {
    await mkdir(this.stateDir, { recursive: true })
    await writeFile(this.tokenPath, `${token}\n`, { mode: 0o600 })
    await chmod(this.tokenPath, 0o600)
  }
}
