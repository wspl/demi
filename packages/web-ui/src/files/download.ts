/**
 * Starts the browser's download of `url`, which the server answers as an
 * attachment (`file-previews.md` § What the user sees): the page stays where
 * it is.
 */
export function downloadUrl(url: string): void {
  const link = document.createElement('a')
  link.href = url
  link.download = ''
  link.click()
}
