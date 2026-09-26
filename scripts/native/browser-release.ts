import { fileArtifact } from './release-files'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { resolve, join } from 'node:path'
import { z } from 'zod'
import { browserReleaseSchema } from '../../packages/browser-protocol/src/index'

const version = z.string().regex(/^\d+\.\d+\.\d+\.\d+$/).parse(process.argv[2])
const directory = resolve('.cache/browser-releases', version)
await mkdir(directory, { recursive: true })
const metadata = z.object({ version: z.literal(version), downloads: z.object({ chrome: z.array(z.object({ platform: z.string(), url: z.url() })) }) })
  .parse(await (await fetch(`https://googlechromelabs.github.io/chrome-for-testing/${version}.json`)).json())
const platforms = [
  { platform: 'mac-arm64', target: 'aarch64-apple-darwin', executable: 'chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing' },
  { platform: 'mac-x64', target: 'x86_64-apple-darwin', executable: 'chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing' },
  { platform: 'linux-arm64', target: 'aarch64-unknown-linux-musl', executable: 'chrome-linux-arm64/chrome' },
  { platform: 'linux64', target: 'x86_64-unknown-linux-musl', executable: 'chrome-linux64/chrome' },
  { platform: 'win64', target: 'x86_64-pc-windows-msvc', executable: 'chrome-win64/chrome.exe' },
]
const records = await Promise.all(platforms.map(async ({ platform, ...record }) => {
  const entry = metadata.downloads.chrome.find(entry => entry.platform === platform)
  if (!entry || new URL(entry.url).origin !== 'https://storage.googleapis.com')
    throw new Error(`No official Chrome archive for ${platform}`)
  const path = join(directory, `${platform}.zip`)
  let bytes: Uint8Array
  try {
    bytes = await readFile(path)
  } catch (error) {
    if (!(error instanceof Error) || !('code' in error) || error.code !== 'ENOENT')
      throw error
    const response = await fetch(entry.url)
    if (!response.ok)
      throw new Error(`Chrome download failed: ${response.status}`)
    bytes = new Uint8Array(await response.arrayBuffer())
    await writeFile(path, bytes, { flag: 'wx' })
  }
  const artifact = await fileArtifact(path)
  console.log(`Verified release archive: ${platform} (${artifact.size} bytes)`)
  return { ...record, url: entry.url, ...artifact }
}))
const release = browserReleaseSchema.parse({ version, platforms: records })
await writeFile(resolve('crates/builtin-protocol/src/release/chrome.json'), `${JSON.stringify(release, null, 2)}\n`)
