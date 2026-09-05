// The image tools (`managed-hosts.md` § Home persistence): e2fsprogs over
// image files, never a mount, never root. Making a home image from a
// directory (`mke2fs -d`), shrinking one to its data after a hibernate
// (`e2fsck -f`, `resize2fs -M`, truncate), growing its backing file (the
// guest grows the filesystem into it, at boot and on `home_grown`).
import { mkdir, mkdtemp, rename, rm, stat, truncate } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { decodeUtf8 } from '@demicodes/utils'

/** The tools a backend with managed hosts needs on its machine; checked once at start. */
export const IMAGE_TOOLS = ['mke2fs', 'e2fsck', 'resize2fs'] as const

export function missingImageTools(): string[] {
  return IMAGE_TOOLS.filter((tool) => Bun.which(tool) === null)
}

export interface ToolResult {
  code: number | null
  stdout: string
  stderr: string
}

/** Runs one tool to its end; the caller judges the exit code (e2fsck's 1 means "corrected"). */
export async function runTool(command: string, args: string[]): Promise<ToolResult> {
  const child = Bun.spawn([command, ...args], { stdout: 'pipe', stderr: 'pipe', stdin: 'ignore' })
  const [stdout, stderr, code] = await Promise.all([
    new Response(child.stdout).arrayBuffer().then((bytes) => decodeUtf8(new Uint8Array(bytes))),
    new Response(child.stderr).arrayBuffer().then((bytes) => decodeUtf8(new Uint8Array(bytes))),
    child.exited,
  ])
  return { code, stdout, stderr }
}

function failed(command: string, result: ToolResult): Error {
  return new Error(`${command} exited ${result.code ?? 'by signal'}: ${(result.stderr || result.stdout).trim()}`)
}

/**
 * The owner's home as an image: `homeDir` becomes `/demi` inside it (the
 * image mounts at `/home`), sized to `nominalBytes`. The directory is
 * consumed. Ownership inside the image is the backend user's; the guest
 * chowns to its user on the first boot (`demi.firstboot`).
 */
export async function makeHomeImage(homeDir: string, imagePath: string, nominalBytes: number): Promise<void> {
  const root = await mkdtemp(join(dirname(imagePath), '.mkhome-'))
  try {
    await rename(homeDir, join(root, 'demi'))
    const result = await runTool('mke2fs', ['-q', '-t', 'ext4', '-F', '-L', 'home', '-d', root, imagePath, `${Math.ceil(nominalBytes / 1024)}k`])
    if (result.code !== 0) throw failed('mke2fs', result)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}

/** `resize2fs -M`'s report: the filesystem's final size. */
export function parseResizedBlocks(output: string): { blocks: number; blockSize: number } | null {
  const match = /is now (\d+) \((\d+)k\) blocks long/.exec(output)
  if (!match) return null
  return { blocks: Number(match[1]), blockSize: Number(match[2]) * 1024 }
}

/**
 * After a `kill -9`: the journal replayed and the filesystem checked, the
 * filesystem shrunk to its data, the file cut to the filesystem. Returns
 * the image's size afterwards.
 */
export async function shrinkImage(imagePath: string): Promise<number> {
  const check = await runTool('e2fsck', ['-fy', imagePath])
  // 0: clean; 1: errors corrected. Anything else is a filesystem the guest cannot be trusted with.
  if (check.code !== 0 && check.code !== 1) throw failed('e2fsck', check)
  const resized = await runTool('resize2fs', ['-M', imagePath])
  if (resized.code !== 0) throw failed('resize2fs', resized)
  const size = parseResizedBlocks(`${resized.stdout}\n${resized.stderr}`)
  if (!size) throw new Error(`resize2fs reported no size: ${(resized.stderr || resized.stdout).trim()}`)
  const bytes = size.blocks * size.blockSize
  await truncate(imagePath, bytes)
  return bytes
}

/** The backing file enlarged to `bytes` (never shrunk here); the guest grows the filesystem into it. */
export async function growImage(imagePath: string, bytes: number): Promise<void> {
  if ((await stat(imagePath)).size >= bytes) return
  await truncate(imagePath, bytes)
}

/** An empty directory for a Cloud workspace's first image, or any home that starts with nothing. */
export async function emptyHomeDir(path: string): Promise<string> {
  await mkdir(path, { recursive: true })
  return path
}
