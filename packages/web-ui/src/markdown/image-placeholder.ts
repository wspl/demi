import { t } from '../files/locales/text'
import { escapeHtml } from './html'

/**
 * What stands in an image's place while its file is read, when its file is
 * missing or when its bytes do not load: its alt text in a quiet dashed box
 * (`.image-placeholder`), as a web browser shows a missing image's
 * description, never the browser's broken-image icon
 * (`file-previews.md` § Markdown). A message and a document show the same.
 */
export function imagePlaceholderHtml(alt: string): string {
  const words = escapeHtml(alt.trim() || t('view.imageFailed'))
  return `<span class="image-placeholder" role="img" aria-label="${words}">${words}</span>`
}
