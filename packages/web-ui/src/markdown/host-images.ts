import { computed, type ComputedRef } from 'vue'
import { hostImage, type HostImage } from '../files/host-image'
import { useShowings } from '../files/showing'
import type { FileContents } from '../files/types'

/**
 * What shows of each Host image at `paths`, by its path: each file's
 * description is shown from the kept files `contents` gives while the calling
 * component lives, so when the Host reports the file changed it is read again
 * and its image shows the new version, wherever it is named. The owner of how
 * the Host image of a Markdown document follows its file; a reply's pictures
 * are copies the backend keeps and read no Host.
 */
export function useHostImagesAt(
  contents: () => FileContents | undefined,
  paths: () => string[],
): ComputedRef<ReadonlyMap<string, HostImage>> {
  const shown = useShowings(contents, paths, (kept, path) => kept.showDescription(path))
  return computed(() => new Map([...shown.entries.value].map(([path, entry]) => [path, hostImage(entry, contents(), path)])))
}
