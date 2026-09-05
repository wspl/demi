import type { HostlessEnvironment, ShellHandover } from '@demicodes/host-virtual'
import type { ShellAbortInput, ShellCommandStatus, ShellEnvironment, ShellExecInput, ShellStatusInput, ShellWriteInput } from '@demicodes/shell'

/** The machine's shell engine: one that takes the hostless shells over under their ids. */
export interface MachineShell extends ShellEnvironment {
  adoptShell(shell: ShellHandover): void
}

export interface Machine {
  environment: MachineShell
  /** The machine's home: what the hostless home stands in for. */
  home: string
}

/**
 * The session upgrade (`sessions-and-targets.md` § Hostless execution), as
 * the shell of a hostless conversation: every exec is decided at parse time
 * before anything runs. A script inside tinybash's subset runs hostless; the
 * first one outside it moves the conversation to a machine — the files, the
 * binding, the shell state — and runs there whole. From then on this object
 * is the session's shell on the machine too: each hostless shell is adopted
 * there under its id the first time it is used, in its directory and with
 * its variables, so nothing the model holds goes stale. Nothing enters the
 * transcript; the model is never told. A provisioning failure is this
 * call's tool error, and the conversation stays hostless for the next one.
 */
export class UpgradingShell implements ShellEnvironment {
  private upgrading: Promise<void> | null = null
  private machine: Machine | null = null
  private readonly adopted = new Set<string>()

  constructor(
    private readonly hostless: HostlessEnvironment,
    /** Provisions and binds the conversation's machine and attaches its shell here; the same instance the session uses afterwards. */
    private readonly upgrade: () => Promise<void>,
    /** The hostless home: what the machine's home stands in for. */
    private readonly hostlessHome: string,
  ) {}

  /** The machine this conversation now runs on — from this shell's own upgrade, or from the session that upgraded first. */
  attach(machine: Machine): void {
    this.machine ??= machine
  }

  async exec(input: ShellExecInput): Promise<ShellCommandStatus> {
    if (this.machine === null) {
      if (this.upgrading === null) {
        if ((await this.hostless.outside(input)) === null) return this.hostless.exec(input)
        const upgrading = this.upgrade().catch((error: unknown) => {
          if (this.upgrading === upgrading) this.upgrading = null
          throw error
        })
        this.upgrading = upgrading
      }
      // An upgrade in flight takes every exec with it: the files are on their way to the machine.
      await this.upgrading
      if (this.machine === null) throw new Error('the conversation moved to a machine, but its shell was not attached')
    }
    const machine = this.machine
    return machine.environment.exec(this.onMachine(machine, input))
  }

  async status(input: ShellStatusInput): Promise<ShellCommandStatus> {
    return this.ownerOfCommand(input.commandId).status(input)
  }

  async write(input: ShellWriteInput): Promise<ShellCommandStatus> {
    return this.ownerOfCommand(input.commandId).write(input)
  }

  async abort(input: ShellAbortInput): Promise<ShellCommandStatus> {
    return this.ownerOfCommand(input.commandId).abort(input)
  }

  async releaseCommand(commandId: string): Promise<boolean> {
    let released = false
    for (const environment of this.environments()) released = (await environment.releaseCommand(commandId)) || released
    return released
  }

  async disposeShell(shellId: string): Promise<boolean> {
    let disposed = false
    for (const environment of this.environments()) disposed = (await environment.disposeShell(shellId)) || disposed
    return disposed
  }

  async disposeAllShells(): Promise<void> {
    for (const environment of this.environments()) await environment.disposeAllShells()
  }

  getShell(shellId: string): { id: string } | null {
    return this.hostless.getShell(shellId) ?? this.machine?.environment.getShell(shellId) ?? null
  }

  hasCommand(commandId: string): boolean {
    return this.environments().some((environment) => environment.hasCommand(commandId))
  }

  /**
   * The exec as the machine's shell takes it: an ephemeral exec starts where
   * the hostless path says on the machine; a hostless shell is adopted under
   * its id — directory and variables carried — the first time it is used
   * there; a shell the machine created itself passes through.
   */
  private onMachine(machine: Machine, input: ShellExecInput): ShellExecInput {
    if (input.ephemeral) return input.cwd === undefined ? input : { ...input, cwd: this.machinePath(input.cwd, machine.home) }
    if (input.shellId && !this.hostless.getShell(input.shellId)) return input
    const handover = this.hostless.handoverOf(input)
    if (!this.adopted.has(handover.shellId)) {
      machine.environment.adoptShell({ ...handover, cwd: this.machinePath(handover.cwd, machine.home) })
      this.adopted.add(handover.shellId)
    }
    return input
  }

  private machinePath(hostlessPath: string, machineHome: string): string {
    if (hostlessPath === this.hostlessHome) return machineHome
    if (hostlessPath.startsWith(`${this.hostlessHome}/`)) return `${machineHome}${hostlessPath.slice(this.hostlessHome.length)}`
    return hostlessPath
  }

  private environments(): ShellEnvironment[] {
    return this.machine ? [this.hostless, this.machine.environment] : [this.hostless]
  }

  private ownerOfCommand(commandId: string): ShellEnvironment {
    for (const environment of this.environments()) if (environment.hasCommand(commandId)) return environment
    throw new Error(`Unknown command "${commandId}"`)
  }
}
