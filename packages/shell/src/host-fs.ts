import type {
  BufferEncoding,
  CpOptions,
  DirentEntry,
  FileContent,
  FsStat,
  IFileSystem,
  MkdirOptions,
  ReadFileOptions,
  RmOptions,
  WriteFileOptions,
} from 'just-bash/fs/interface'
import { concatBytes, decodeUtf8, encodeUtf8 } from './bytes'
import type { Host } from './host'

const UNSUPPORTED = (op: string): Error => new Error(`HostBackedFileSystem: ${op} is not supported`)

export class HostBackedFileSystem implements IFileSystem {
  constructor(private readonly host: Host) {}

  async readFile(path: string, _options?: ReadFileOptions | BufferEncoding): Promise<string> {
    const { stdout } = await this.run(['cat', '--', path], '')
    return decodeUtf8(stdout)
  }

  async readFileBuffer(path: string): Promise<Uint8Array> {
    const { stdout } = await this.run(['cat', '--', path], '')
    return stdout
  }

  async writeFile(path: string, content: FileContent, _options?: WriteFileOptions | BufferEncoding): Promise<void> {
    const bytes = typeof content === 'string' ? encodeUtf8(content) : content
    await this.run(['tee', '--', path], bytes)
  }

  async appendFile(path: string, content: FileContent, _options?: WriteFileOptions | BufferEncoding): Promise<void> {
    const bytes = typeof content === 'string' ? encodeUtf8(content) : content
    await this.run(['tee', '-a', '--', path], bytes)
  }

  async exists(path: string): Promise<boolean> {
    const { exitCode } = await this.run(['test', '-e', path], '')
    return exitCode === 0
  }

  async stat(path: string): Promise<FsStat> {
    const fileCheck = await this.run(['test', '-f', path], '')
    if (fileCheck.exitCode === 0) {
      return emptyStat({ isFile: true })
    }
    const dirCheck = await this.run(['test', '-d', path], '')
    if (dirCheck.exitCode === 0) {
      return emptyStat({ isDirectory: true })
    }
    const linkCheck = await this.run(['test', '-L', path], '')
    if (linkCheck.exitCode === 0) {
      return emptyStat({ isSymbolicLink: true })
    }
    throw new Error(`HostBackedFileSystem: stat: no such file or directory: '${path}'`)
  }

  async readdir(path: string): Promise<string[]> {
    const { stdout, exitCode } = await this.run(['ls', '-1', '-A', '--', path], '')
    if (exitCode !== 0) {
      throw new Error(`HostBackedFileSystem: readdir: cannot access '${path}'`)
    }
    const text = decodeUtf8(stdout)
    return text.split('\n').filter((line) => line.length > 0)
  }

  async readdirWithFileTypes(path: string): Promise<DirentEntry[]> {
    const names = await this.readdir(path)
    const entries: DirentEntry[] = []
    for (const name of names) {
      const fullPath = joinPath(path, name)
      const stat = await this.stat(fullPath)
      entries.push({
        name,
        isFile: stat.isFile,
        isDirectory: stat.isDirectory,
        isSymbolicLink: stat.isSymbolicLink,
      })
    }
    return entries
  }

  resolvePath(base: string, path: string): string {
    if (path.startsWith('/')) return path
    if (base.endsWith('/')) return `${base}${path}`
    return `${base}/${path}`
  }

  getAllPaths(): string[] {
    return []
  }

  async mkdir(_path: string, _options?: MkdirOptions): Promise<void> {
    throw UNSUPPORTED('mkdir')
  }

  async rm(_path: string, _options?: RmOptions): Promise<void> {
    throw UNSUPPORTED('rm')
  }

  async cp(_src: string, _dest: string, _options?: CpOptions): Promise<void> {
    throw UNSUPPORTED('cp')
  }

  async mv(_src: string, _dest: string): Promise<void> {
    throw UNSUPPORTED('mv')
  }

  async chmod(_path: string, _mode: number): Promise<void> {
    throw UNSUPPORTED('chmod')
  }

  async symlink(_target: string, _linkPath: string): Promise<void> {
    throw UNSUPPORTED('symlink')
  }

  async link(_existingPath: string, _newPath: string): Promise<void> {
    throw UNSUPPORTED('link')
  }

  async readlink(_path: string): Promise<string> {
    throw UNSUPPORTED('readlink')
  }

  async lstat(path: string): Promise<FsStat> {
    const linkCheck = await this.run(['test', '-L', path], '')
    if (linkCheck.exitCode === 0) {
      return emptyStat({ isSymbolicLink: true })
    }
    return this.stat(path)
  }

  async realpath(path: string): Promise<string> {
    const { stdout, exitCode } = await this.run(['readlink', '-f', '--', path], '')
    if (exitCode !== 0) {
      throw new Error(`HostBackedFileSystem: realpath: cannot resolve '${path}'`)
    }
    return decodeUtf8(stdout).replace(/\n$/, '')
  }

  async utimes(_path: string, _atime: Date, _mtime: Date): Promise<void> {
    throw UNSUPPORTED('utimes')
  }

  private async run(args: string[], stdin: Uint8Array | string): Promise<{ stdout: Uint8Array; stderr: Uint8Array; exitCode: number | null }> {
    const stdinBytes = typeof stdin === 'string' ? encodeUtf8(stdin) : stdin
    const handle = await this.host.spawn({ command: args[0], args: args.slice(1), cwd: this.host.root })
    if (stdinBytes.byteLength > 0) {
      await handle.writeStdin(stdinBytes)
    }
    await handle.closeStdin()
    const [stdoutChunks, stderrChunks, exit] = await Promise.all([
      collectBytes(handle.stdout),
      collectBytes(handle.stderr),
      handle.wait(),
    ])
    return {
      stdout: concatBytes(stdoutChunks),
      stderr: concatBytes(stderrChunks),
      exitCode: exit.exitCode,
    }
  }
}

function emptyStat(overrides: Partial<FsStat>): FsStat {
  return {
    isFile: false,
    isDirectory: false,
    isSymbolicLink: false,
    mode: 0o644,
    size: 0,
    mtime: new Date(0),
    ...overrides,
  }
}

function joinPath(base: string, name: string): string {
  if (base.endsWith('/')) return `${base}${name}`
  return `${base}/${name}`
}

async function collectBytes(stream: AsyncIterable<Uint8Array>): Promise<Uint8Array[]> {
  const chunks: Uint8Array[] = []
  for await (const chunk of stream) chunks.push(chunk)
  return chunks
}
