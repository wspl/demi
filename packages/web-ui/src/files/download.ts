import { showToast } from '../infra/toast'
import { pageLog } from '../infra/log'
import { t } from './locales/text'

/**
 * Starts the download of `url`, which the server answers as an attachment:
 * the page stays where it is. `name` names the file where the server does
 * not, as for a picture saved from a message.
 *
 * Demi added to an iPhone's or an iPad's Home Screen cannot download: a
 * download there covers the whole app with the file, with no way back. There
 * the file goes to the share sheet, whose Save Image and Save to Files keep
 * it (`shareFile`). Safari in a tab, and every other browser, downloads it.
 */
export function downloadUrl(url: string, name = ''): void {
  if (onHomeScreen()) {
    void shareFile(url, name)
    return
  }
  const link = document.createElement('a')
  link.href = url
  link.download = name
  link.click()
}

/** Whether the page is Demi added to the Home Screen of an iPhone or an iPad, Safari's `navigator.standalone`. */
function onHomeScreen(): boolean {
  return Reflect.get(navigator, 'standalone') === true
}

/**
 * Hands the file at `url` to the share sheet. The sheet opens only within a
 * few seconds of the press that asked for it, so a file that took longer to
 * arrive waits in a toast whose Save opens the sheet with its own press. A
 * sheet the user closes saves nothing and says nothing.
 */
async function shareFile(url: string, name: string): Promise<void> {
  let file: File
  try {
    const response = await fetch(url)
    if (!response.ok)
      throw new Error(`the file answered ${response.status}`)
    const blob = await response.blob()
    const named = name || answeredName(response) || url.split(/[/?#]/).filter(Boolean).at(-1) || 'download'
    file = new File([blob], withExtension(named, blob.type), { type: blob.type })
  } catch (error) {
    pageLog.warn('download.failed', { reason: error instanceof Error ? error.message : String(error) })
    showToast({ title: t('download.failed'), message: name || undefined, tone: 'danger' })
    return
  }
  const share = (): void => {
    // A page served without HTTPS has no share sheet, and can save nothing there.
    const shared = typeof navigator.share === 'function'
      ? navigator.share({ files: [file] })
      : Promise.reject(new Error('the page has no share sheet'))
    shared.catch((error: unknown) => {
      if (error instanceof DOMException && error.name === 'AbortError')
        return
      if (error instanceof DOMException && error.name === 'NotAllowedError') {
        showToast({ title: t('download.ready'), message: file.name, tone: 'neutral', oneLine: true, action: { label: t('download.save'), run: share } })
        return
      }
      pageLog.warn('download.failed', { reason: error instanceof Error ? error.message : String(error) })
      showToast({ title: t('download.failed'), message: file.name, tone: 'danger' })
    })
  }
  share()
}

/**
 * `name` with the extension of `mediaType` when it has none, as a picture
 * named by its description: the share sheet tells a picture by its name's
 * extension, and offers only Save to Files for a name without one.
 */
function withExtension(name: string, mediaType: string): string {
  if (/\.[a-z0-9]{1,5}$/i.test(name))
    return name
  const subtype = /^[a-z]+\/([a-z0-9.+-]+)/i.exec(mediaType)?.[1]?.toLowerCase()
  if (!subtype)
    return name
  const extension = { jpeg: 'jpg', 'svg+xml': 'svg', quicktime: 'mov', mpeg: mediaType.startsWith('audio/') ? 'mp3' : 'mpg' }[subtype] ?? subtype
  return /^[a-z0-9]{1,5}$/.test(extension) ? `${name}.${extension}` : name
}

/** The file name a server's `Content-Disposition` gives, or null. */
function answeredName(response: Response): string | null {
  const disposition = response.headers.get('content-disposition') ?? ''
  const encoded = /filename\*=UTF-8''([^;]+)/i.exec(disposition)?.[1]
  if (encoded) {
    try {
      return decodeURIComponent(encoded)
    } catch {
      // A name that does not decode falls back to the plain one.
    }
  }
  return /filename="?([^";]+)"?/i.exec(disposition)?.[1] ?? null
}
