import { join, normalize } from 'node:path'

export function createStaticHandler(distDir: string): (pathname: string) => Promise<Response> {
  const indexPath = join(distDir, 'index.html')

  return async function serve(pathname: string): Promise<Response> {
    const requested = pathname === '/' ? '/index.html' : pathname
    const safeRelative = normalize(requested).replace(/^(\.\.(?:[/\\]|$))+/, '')
    const file = Bun.file(join(distDir, safeRelative))
    if (await file.exists()) return new Response(file)

    const index = Bun.file(indexPath)
    if (await index.exists()) return new Response(index, { headers: { 'content-type': 'text/html' } })

    return new Response('Demi web build not found. Run `vite build` first.', { status: 404 })
  }
}
