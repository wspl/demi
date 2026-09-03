import type { HostlessEnvironment } from '@demicodes/host-virtual'
import { shellQuote, type ShellAbortInput, type ShellCommandStatus, type ShellEnvironment, type ShellExecInput, type ShellStatusInput, type ShellWriteInput } from '@demicodes/shell'

/**
 * The session upgrade (`sessions-and-targets.md` § Hostless execution), as
 * the shell of a hostless conversation: every exec is decided at parse time
 * before anything runs. A script inside tinybash's subset runs hostless; the
 * first one outside it moves the conversation to a machine — the files, the
 * binding, the shell state — and runs there whole. Nothing enters the
 * transcript; the model is never told. A provisioning failure is this
 * call's tool error, and the conversation stays hostless for the next one.
 */
export interface Machine {
  environment: ShellEnvironment
  home: string
}

export class UpgradingShell implements ShellEnvironment {
  private machine: Promise<Machine> | null = null

  constructor(
    private readonly hostless: HostlessEnvironment,
    /** Provisions and binds the machine and returns its shell environment — the same instance the session uses afterwards — and its home. */
    private readonly upgrade: () => Promise<Machine>,
    /** The hostless home: what the machine's home stands in for. */
    private readonly hostlessHome: string,
  ) {}

  async exec(input: ShellExecInput): Promise<ShellCommandStatus> {
    if (this.machine === null && (await this.hostless.outside(input)) === null) return this.hostless.exec(input)
    const handover = this.machine === null ? this.hostless.handoverOf(input) : null
    if (this.machine === null) {
      this.machine = this.upgrade().catch((error: unknown) => {
        this.machine = null
        throw error
      })
    }
    const machine = await this.machine
    // The first job continues where tinybash's shell stood: its working
    // directory (the hostless home is the machine's home) and the variables
    // the session set; a machine's shell carries its cwd between jobs by
    // itself afterwards.
    const script = handover ? `${handoverPrefix({ ...handover, cwd: this.machinePath(handover.cwd, machine.home) })}${input.script}` : input.script
    return machine.environment.exec({ ...input, script })
  }

  async status(input: ShellStatusInput): Promise<ShellCommandStatus> {
    return (await this.ownerOfCommand(input.commandId)).status(input)
  }

  async write(input: ShellWriteInput): Promise<ShellCommandStatus> {
    return (await this.ownerOfCommand(input.commandId)).write(input)
  }

  async abort(input: ShellAbortInput): Promise<ShellCommandStatus> {
    return (await this.ownerOfCommand(input.commandId)).abort(input)
  }

  async releaseCommand(commandId: string): Promise<boolean> {
    for (const environment of await this.environments()) if (await environment.releaseCommand(commandId)) return true
    return false
  }

  async disposeShell(shellId: string): Promise<boolean> {
    for (const environment of await this.environments()) if (await environment.disposeShell(shellId)) return true
    return false
  }

  async disposeAllShells(): Promise<void> {
    for (const environment of await this.environments()) await environment.disposeAllShells()
  }

  getShell(shellId: string): { id: string } | null {
    return this.hostless.getShell(shellId)
  }

  hasCommand(commandId: string): boolean {
    return this.hostless.hasCommand(commandId)
  }

  private machinePath(hostlessPath: string, machineHome: string): string {
    if (hostlessPath === this.hostlessHome) return machineHome
    if (hostlessPath.startsWith(`${this.hostlessHome}/`)) return `${machineHome}${hostlessPath.slice(this.hostlessHome.length)}`
    return hostlessPath
  }

  private async environments(): Promise<ShellEnvironment[]> {
    const machine = this.machine ? await this.machine.catch(() => null) : null
    return machine ? [this.hostless, machine.environment] : [this.hostless]
  }

  private async ownerOfCommand(commandId: string): Promise<ShellEnvironment> {
    for (const environment of await this.environments()) if (environment.hasCommand(commandId)) return environment
    throw new Error(`Unknown command "${commandId}"`)
  }
}

function handoverPrefix(handover: { cwd: string; vars: Record<string, string> }): string {
  const parts = [`cd ${shellQuote(handover.cwd)}`]
  for (const [key, value] of Object.entries(handover.vars)) parts.push(`${key}=${shellQuote(value)}`)
  return `${parts.join(' && ')}\n`
}
