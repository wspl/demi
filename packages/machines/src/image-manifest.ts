import { z } from 'zod'
import { nativeArtifactSchema, nativePackageSchema } from '@demicodes/command-protocol'
import { runnerReleaseSchema } from '@demicodes/runner-protocol/release'

const file = nativeArtifactSchema
export const cloudImageManifestSchema = z.strictObject({
  formatVersion: z.literal(1),
  os: z.literal('linux'),
  architecture: z.enum(['amd64', 'arm64']),
  rootfs: file.extend({ file: z.literal('rootfs.tar.zst') }),
  ubuntu: z.string().min(1),
  packages: z.array(z.strictObject({ name: z.string().min(1), version: z.string().min(1) })),
  executables: z.record(z.string().regex(/^\/(usr|opt)\/[^\r\n]+$/).refine(path => !path.split('/').includes('..')), file),
  releases: z.array(nativePackageSchema),
  runner: runnerReleaseSchema,
  tools: z.array(z.strictObject({ name: z.string().min(1), version: z.string().min(1), sha256: z.string().regex(/^[a-f0-9]{64}$/) })),
}).superRefine((manifest, context) => {
  const target = manifest.architecture === 'arm64' ? 'aarch64-unknown-linux-musl' : 'x86_64-unknown-linux-musl'
  const runner = manifest.runner.targets[target]
  const executable = manifest.executables['/usr/bin/demi-runner']
  if (!runner || !executable || runner.sha256 !== executable.sha256 || runner.size !== executable.size) {
    context.addIssue({ code: 'custom', path: ['runner'], message: 'Runner release must identify the embedded executable' })
  }
  for (const release of manifest.releases) {
    const artifact = release.targets[target]
    if (!artifact || !Object.entries(manifest.executables).some(([path, entry]) =>
      path.startsWith(`/opt/demi/artifacts/${artifact.sha256}/`) && entry.sha256 === artifact.sha256 && entry.size === artifact.size)) {
      context.addIssue({ code: 'custom', path: ['releases'], message: `Missing embedded artifact for ${release.id}` })
    }
  }
})
export type CloudImageManifest = z.infer<typeof cloudImageManifestSchema>
