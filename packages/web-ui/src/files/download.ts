/**
 * Starts the web browser's download of `url`, which the server answers as
 * an attachment: the page stays where it is. `name` names the file where the
 * server does not, as for a picture saved from a message.
 */
export function downloadUrl(url: string, name = ''): void {
  const link = document.createElement('a')
  link.href = url
  link.download = name
  link.click()
}
