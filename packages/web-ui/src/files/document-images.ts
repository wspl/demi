import { nextTick, watch } from 'vue'
import { HOST_FILE_ATTRIBUTE } from '../markdown/document'
import { useHostImagesAt } from '../markdown/host-images'
import type { FileContents } from './types'

/**
 * The images of a Markdown document the page shows (`file-previews.md`
 * § Markdown, § Files named in messages), in the element `root` holds the
 * rendering `html` in.
 *
 * Each Host image of the document (`hostFiles` are their paths) is given the
 * address of its file's version the page keeps, where it stands, so a new
 * version loads in place and nothing else of the document is rendered
 * again; one whose file is still being read, or is not there, shows its alt
 * text.
 *
 * An image whose bytes do not load, as while the Host is away, or whose file
 * is not there, shows its alt text in a quiet box in its place, as a web
 * browser shows a missing image's description, instead of its broken picture
 * glyph. Each box keeps its image, which tries again when the document is
 * read again (`reload`) and, for a Host image, when its file has a version it
 * has not tried.
 */
export function useDocumentImages(options: {
  root: () => HTMLElement | undefined
  html: () => string
  hostFiles: () => string[]
  contents: () => FileContents | undefined
}): { onError(event: Event): void; reload(): void } {
  const { html } = options
  const hostImages = useHostImagesAt(options.contents, options.hostFiles)
  const failed = new Map<HTMLElement, HTMLImageElement>()

  /** Shows `image`'s alt text in its place. */
  function fail(image: HTMLImageElement): void {
    const placeholder = document.createElement('span')
    placeholder.className = 'inline-flex max-w-full items-center rounded-md border border-dashed border-line px-2 py-0.5 align-middle text-[12px] text-fg-muted'
    const words = image.alt.trim() || 'Could not show this image.'
    placeholder.setAttribute('role', 'img')
    placeholder.setAttribute('aria-label', words)
    placeholder.textContent = words
    failed.set(placeholder, image)
    image.replaceWith(placeholder)
  }

  function onError(event: Event): void {
    if (event.target instanceof HTMLImageElement)
      fail(event.target)
  }

  function reload(): void {
    for (const [placeholder, image] of failed) {
      // A Host image whose file is not there has no address to try again.
      if (!image.hasAttribute('src'))
        continue
      placeholder.replaceWith(image)
      // Setting the address again loads it again.
      image.src = image.src
      failed.delete(placeholder)
    }
  }

  // A new rendering brings its own images.
  watch(html, () => failed.clear())

  async function address(): Promise<void> {
    // The rendering reaches the page first.
    await nextTick()
    const placeholders = new Map([...failed].map(([placeholder, image]) => [image, placeholder]))
    const images = [...options.root()?.querySelectorAll<HTMLImageElement>(`img[${HOST_FILE_ATTRIBUTE}]`) ?? [], ...placeholders.keys()]
    for (const image of images) {
      const shown = hostImages.value.get(image.getAttribute(HOST_FILE_ATTRIBUTE) ?? '')
      const src = shown?.state === 'shown' ? shown.src : null
      const placeholder = placeholders.get(image)
      // A file that is not there shows its alt text, as an image that does not load does.
      if (shown?.state === 'none') {
        if (!placeholder)
          fail(image)
        continue
      }
      if (placeholder) {
        // A failed image tries again only for a version it has not tried.
        if (src === null || src === image.getAttribute('src'))
          continue
        placeholder.replaceWith(image)
        failed.delete(placeholder)
      }
      if (src === null)
        image.removeAttribute('src')
      else
        image.setAttribute('src', src)
    }
  }
  watch([html, hostImages], address, { immediate: true, flush: 'post' })

  return { onError, reload }
}
