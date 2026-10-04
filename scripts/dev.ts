// Runs the one-command development backend (backend.md § One-command
// development backend) with the arguments given. Bun reads the repository's
// .env into this process's environment but not into a package.json script's,
// so this passes it on: the DEMI_DEV_PROVIDER_* variables reach the command.
import { resolve } from 'node:path'

const dev = Bun.spawn(['cargo', 'xtask', 'dev', ...Bun.argv.slice(2)], {
  cwd: resolve(import.meta.dir, '..'),
  env: process.env,
  stdio: ['inherit', 'inherit', 'inherit'],
})
// An interrupt or a stop reaches the command, which stops what it started;
// this ends when it does.
for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, () => dev.kill(signal))
}
process.exit(await dev.exited)
