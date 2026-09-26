// Host-side ext4 image operations for persistent Cloud volumes.
import { chmod, mkdir, mkdtemp, rename, rm, stat } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { z } from 'zod'

/**
 * The tools a backend with managed hosts needs on its machine; checked once at
 * start.
 */
export const IMAGE_TOOLS = ['mke2fs', 'e2fsck', 'resize2fs', 'dumpe2fs', 'losetup', 'mount', 'mountpoint', 'umount', 'fsfreeze', 'ip', 'nft', 'bsdtar', 'sync', 'chown', 'sysctl', 'nsenter', 'cp'] as const

export function missingImageTools(): string[] {
  return IMAGE_TOOLS.filter((tool) => Bun.which(tool) === null)
}

export interface ToolResult {
  code: number | null
  stdout: string
  stderr: string
}

/**
 * Runs one tool to its end; the caller judges the exit code (e2fsck's 1 means
 * "corrected").
 */
export async function runTool(
  command: string,
  args: string[],
  input?: string,
  timeout = 60_000,
): Promise<ToolResult> {
  const child = Bun.spawn([command, ...args], {
    stdout: 'pipe',
    stderr: 'pipe',
    stdin: input === undefined ? 'ignore' : new Blob([input]),
    timeout,
    env: { ...process.env, LC_ALL: 'C.UTF-8' },
    killSignal: 'SIGKILL'
  })
  const [stdout, stderr, code] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ])
  return { code, stdout, stderr }
}

export function failed(command: string, result: ToolResult): Error {
  return new Error(
    `${command} exited ${result.code ?? 'by signal'}: ${(result.stderr || result.stdout).trim()}`
  )
}

/** Create /home from a prepared, UID 1000-owned home directory. */
export async function makeHomeImage(
  homeDir: string,
  imagePath: string,
  nominalBytes: number
): Promise<void> {
  const root = await mkdtemp(join(dirname(imagePath), '.mkhome-'))
  try {
    await rename(homeDir, join(root, 'demi'))
    const result = await runTool('mke2fs', [
      '-q',
      '-t',
      'ext4',
      '-F',
      '-L',
      'home',
      '-d',
      root,
      imagePath,
      `${Math.ceil(nominalBytes / 1024)}k`
    ])
    if (result.code !== 0)
      throw failed('mke2fs', result)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}

/**
 * Empty persistent overlay volume; the manager creates upper/work before use.
 */
export async function makeSystemImage(
  path: string,
  bytes: number
): Promise<void> {
  const result = await runTool('mke2fs', [
    '-q',
    '-t',
    'ext4',
    '-F',
    '-L',
    'system',
    path,
    `${Math.ceil(bytes / 1024)}k`
  ])
  if (result.code !== 0)
    throw failed('mke2fs', result)
}

/** Require a successful Cloud infrastructure operation. */
export async function requireTool(command: string, args: string[], input?: string, timeout?: number): Promise<string> {
  const result = await runTool(command, args, input, timeout)
  if (result.code !== 0) throw failed(command, result)
  return result.stdout.trim()
}

/** Mount the Cloud system overlay with the fixed persistent image layout. */
export async function mountSystemOverlay(base: string, volume: string, target: string): Promise<void> {
  const upper = join(volume, 'upper')
  if (await mkdir(upper, { recursive: true })) {
    // OverlayFS exposes the upper directory's mode as the sandbox root mode.
    // Infrastructure's restrictive umask must not make a new root inaccessible.
    await chmod(upper, 0o755)
  }
  await mkdir(join(volume, 'work'), { recursive: true, mode: 0o700 })
  await requireTool('mount', ['-t', 'overlay', 'overlay', '-o',
    `lowerdir=${base},upperdir=${upper},workdir=${join(volume, 'work')},index=off,metacopy=off,redirect_dir=off`, target])
}

/** Thaw a Cloud filesystem, including recovery after an interrupted freeze. */
export async function thawFilesystem(mount: string): Promise<void> {
  const result = await runTool('fsfreeze', ['--unfreeze', mount])
  // EINVAL means the mounted filesystem was not frozen when recovery reached it.
  if (result.code !== 0 && !result.stderr.includes('Invalid argument')) {
    throw new Error(`Cannot thaw Cloud filesystem ${mount}: ${result.stderr}`)
  }
}

/** Read the actual ext4 capacity after growth or recovery of a Cloud volume. */
export async function volumeBytes(image: string): Promise<number> {
  const output = await requireTool('dumpe2fs', ['-h', image])
  const count = z.coerce.number().int().positive().parse(/^Block count:\s+(\d+)$/m.exec(output)?.[1])
  const size = z.coerce.number().int().positive().parse(/^Block size:\s+(\d+)$/m.exec(output)?.[1])
  return z.number().int().positive().safe().parse(count * size)
}

/** Recover an unmounted Cloud filesystem, completing any interrupted growth. */
export async function recoverVolume(image: string): Promise<number> {
  const result = await runTool('e2fsck', ['-p', image])
  if (result.code !== 0 && result.code !== 1) throw failed('e2fsck', result)
  if ((await stat(image)).size > await volumeBytes(image)) {
    const forced = await runTool('e2fsck', ['-pf', image])
    if (forced.code !== 0 && forced.code !== 1) throw failed('e2fsck', forced)
    await requireTool('resize2fs', [image])
  }
  await requireTool('sync', ['-f', image])
  return volumeBytes(image)
}
