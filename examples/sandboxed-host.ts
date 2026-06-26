// Sandboxing a Host — the security boundary for running agents.
//
//   bun run examples/sandboxed-host.ts
//
// A Host is the only thing standing between an agent and the machine, so it is
// where you enforce a sandbox. Wrap any Host to apply your policy. Here we
// allowlist which commands the agent's shell may spawn; filesystem jailing
// follows the exact same decorator pattern (wrap `fs`, resolve paths against a
// root, reject escapes). See SECURITY.md and docs/guides/implement-a-host.md.
import { LocalHost } from '@demi/host-local'
import type { Host } from '@demi/shell'

/** Wraps a Host so `process.spawn` only runs allowlisted commands; everything else passes through. */
export function commandAllowlistHost(inner: Host, allowed: Iterable<string>): Host {
  const allow = new Set(allowed)
  return {
    ...inner,
    process: {
      spawn: (params) => {
        if (!allow.has(params.command)) {
          throw new Error(`sandbox: command "${params.command}" is not on the allowlist`)
        }
        return inner.process.spawn(params)
      },
    },
  }
}

async function main(): Promise<void> {
  const host = commandAllowlistHost(new LocalHost(process.cwd()), ['echo'])

  // Allowed — runs normally.
  const handle = await host.process.spawn({ command: 'echo', args: ['hello from the sandbox'] })
  const exit = await handle.wait()
  console.log(`echo allowed (exit ${exit.exitCode})`)

  // Denied — the policy throws before anything runs.
  try {
    await host.process.spawn({ command: 'rm', args: ['-rf', '.'] })
    console.log('rm unexpectedly allowed!')
  } catch (error) {
    console.log(`rm denied: ${(error as Error).message}`)
  }
}

void main()
