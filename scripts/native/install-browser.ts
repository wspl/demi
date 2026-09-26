import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { fileArtifact } from './release-files'
import { join, resolve } from 'node:path'
import { z } from 'zod'
import { browserInstallationSchema, browserReleaseSchema } from '../../packages/browser-protocol/src/index'

const root = z.string().min(1).parse(process.argv[2])
const arch = z.enum(['aarch64', 'x86_64']).parse(process.argv[3])
const release = browserReleaseSchema.parse(JSON.parse(await readFile(new URL('../../crates/builtin-protocol/src/release/chrome.json', import.meta.url), 'utf8')))
const record = release.platforms.find(platform => platform.target === `${arch}-unknown-linux-musl`)
if (!record) throw new Error(`Chrome release is unavailable for ${arch}`)
const cache = resolve(import.meta.dir, '../../.cache/browser-releases', release.version)
const archive = join(cache, arch === 'aarch64' ? 'linux-arm64.zip' : 'linux64.zip')
await mkdir(cache, { recursive: true })
if (!await Bun.file(archive).exists()) {
  const response = await fetch(record.url, { redirect: 'error' })
  if (!response.ok) throw new Error(`Chrome download failed: ${response.status}`)
  await Bun.write(archive, response)
}
const artifact = await fileArtifact(archive)
if (artifact.size !== record.size || artifact.sha256 !== record.sha256)
  throw new Error('Chrome archive does not match the pinned release')
const destination = join(resolve(root), 'opt/demi/browsers', record.sha256)
await mkdir(destination, { recursive: true })
// The build host already requires Python. Its standard ZIP extractor owns path
// sanitization and CRC checks; it does not restore executable permission bits.
const unpack = Bun.spawn(['python3', '-c', `
import os
import sys
import zipfile
with zipfile.ZipFile(sys.argv[1]) as archive:
    for entry in archive.infolist():
        path = archive.extract(entry, sys.argv[2])
        mode = (entry.external_attr >> 16) & 0o777
        if mode:
            os.chmod(path, mode)
`, archive, destination], { stdout: 'inherit', stderr: 'inherit' })
if (await unpack.exited !== 0) throw new Error('Chrome extraction failed')
const executable = await fileArtifact(join(destination, record.executable))
const receipt = browserInstallationSchema.parse({ archiveHash: record.sha256, executableHash: executable.sha256 })
await writeFile(join(destination, 'receipt.json'), `${JSON.stringify(receipt)}\n`)
console.log(`Installed Chrome for Testing ${release.version} for ${arch}`)
