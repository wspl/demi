import { chmod, chown, readFile, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { basename, dirname, join } from 'node:path'

interface Scenario {
  directory: string
  mode: 'run' | 'fail' | 'no-pid' | 'no-socket' | 'hold-parent'
}

async function runVm(config: Scenario, root: string, executable: string, uid: number, gid: number): Promise<void> {
  const run = join(root, 'run')
  await chown(root, uid, gid)
  await chown(run, uid, gid)
  await chmod(root, 0o700)
  await chmod(run, 0o700)
  await writeFile(join(root, `${basename(executable)}.pid`), String(process.pid))
  await writeFile(join(config.directory, 'vm.json'), JSON.stringify({ pid: process.pid }))

  if (config.mode === 'no-socket') {
    setInterval(() => {}, 1_000)
    return
  }

  const socket = join(run, 'firecracker.socket')
  const server = createServer(connection => {
    connection.once('data', data => {
      if (data.toString() !== 'exit') throw new Error('unexpected fake VM request')
      connection.end()
      server.close(() => process.exit(0))
    })
  })
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject)
    server.listen(socket, resolve)
  })
  await chown(socket, uid, gid)
  await chmod(socket, 0o600)
}

if (process.argv[2] === '--fake-vm') {
  const [configPath, root, executable, uid, gid] = process.argv.slice(3)
  const config: Scenario = JSON.parse(await readFile(configPath!, 'utf8'))
  await runVm(config, root!, executable!, Number(uid), Number(gid))
} else {
  const configPath = join(dirname(process.argv[1]!), 'scenario.json')
  const config: Scenario = JSON.parse(await readFile(configPath, 'utf8'))
  const args = process.argv.slice(2)
  await writeFile(join(config.directory, 'launcher.json'), JSON.stringify({ pid: process.pid, args }))
  if (config.mode === 'fail') process.exit(23)
  if (config.mode === 'no-pid') process.exit(0)

  const flag = (name: string): string => {
    const index = args.indexOf(name)
    if (index < 0 || args[index + 1] === undefined) throw new Error(`missing fake jailer argument ${name}`)
    return args[index + 1]!
  }
  const executable = flag('--exec-file')
  const root = join(flag('--chroot-base-dir'), basename(executable), flag('--id'), 'root')
  const child = Bun.spawn([
    process.execPath,
    import.meta.path,
    '--fake-vm',
    configPath,
    root,
    executable,
    flag('--uid'),
    flag('--gid'),
  ], { stdin: 'ignore', stdout: 'ignore', stderr: 'inherit' })
  child.unref()
  if (config.mode === 'hold-parent') {
    setInterval(() => {}, 1_000)
  } else {
    process.exit(0)
  }
}
