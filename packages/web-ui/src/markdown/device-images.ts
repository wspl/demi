import { computed, type ComputedRef } from 'vue'
import { deviceImage, type DeviceImage } from '../files/device-image'
import { useShowings } from '../files/showing'
import type { FileContents } from '../files/types'

/**
 * What shows of each device image at `paths`, by its path: each file's
 * description is shown from the kept files `contents` gives while the calling
 * component lives, so when the device reports the file changed it is read again
 * and its image shows the new version, wherever it is named. The owner of how
 * the device image of a Markdown document follows its file; a reply's pictures
 * are copies the backend keeps and read no device.
 */
export function useDeviceImagesAt(
  contents: () => FileContents | undefined,
  paths: () => string[],
): ComputedRef<ReadonlyMap<string, DeviceImage>> {
  const shown = useShowings(contents, paths, (kept, path) => kept.showDescription(path))
  return computed(() => new Map([...shown.entries.value].map(([path, entry]) => [path, deviceImage(entry, contents(), path)])))
}
