import { z } from 'zod'

// Package exports are a recursive package.json format, including fallback arrays.
type PackageExports =
  string | null | PackageExports[] | { [key: string]: PackageExports }
const packageExportsSchema: z.ZodType<PackageExports> = z.lazy(() =>
  z.union([
    z.string(),
    z.null(),
    z.array(packageExportsSchema),
    z.record(z.string(), packageExportsSchema),
  ]),
)
const dependenciesSchema = z.record(z.string(), z.string())
export const packageManifestSchema = z.looseObject({
  name: z.string().min(1),
  version: z.string().min(1),
  private: z.boolean().optional(),
  exports: packageExportsSchema.optional(),
  dependencies: dependenciesSchema.optional(),
  devDependencies: dependenciesSchema.optional(),
  peerDependencies: dependenciesSchema.optional(),
  optionalDependencies: dependenciesSchema.optional(),
})
export type PackageManifest = z.infer<typeof packageManifestSchema>
export const workspaceManifestSchema = z.object({
  workspaces: z.array(z.string().min(1)),
})
export const registryVersionsSchema = z.object({
  versions: z.record(z.string(), z.unknown()),
})
