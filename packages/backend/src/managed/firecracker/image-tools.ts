// Ext4 image creation and backing-file growth. The guest resizes the mounted
// filesystem after the Firecracker drive capacity changes.
import { mkdtemp, rename, rm, stat, truncate } from 'node:fs/promises'
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

/** The backing file enlarged to `bytes` (never shrunk here); the guest grows the filesystem into it. */
export async function growImage(imagePath: string, bytes: number): Promise<void> {
  if ((await stat(imagePath)).size >= bytes) return
  await truncate(imagePath, bytes)
}

/** Empty persistent overlay volume, containing upper/work directories created by guest init. */
export async function makeSystemImage(path: string, bytes: number): Promise<void> {
  const result = await runTool('mke2fs', ['-q', '-t', 'ext4', '-F', '-L', 'system', path, `${Math.ceil(bytes / 1024)}k`])
  if (result.code !== 0) throw failed('mke2fs', result)
}
