import { nativePackageSchema } from '@demicodes/command-protocol'
import descriptor from './release.json'

/** The exact six-target release; executables are supplied through the artifact resolver. */
export const demiPackage = nativePackageSchema.parse(descriptor)
