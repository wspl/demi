/**
 * The web preview's behavior laboratory's sites (`preview.md` § Tests): the
 * spike's lab, served on loopback as a development page and two other
 * sites, `http://localhost:<app>`, `http://auth.localhost:<aux>` and
 * `http://other.localhost:<aux>`, so a Host's engine reaches them by name
 * as a browser does. The spike served its other sites over HTTPS under
 * `*.upstream.test`; the lab's files name them, and each file is served
 * with the names this lab uses.
 */
import http from 'node:http'
import { WebSocketServer } from 'ws'
import { handleLab } from './routes'

export interface LabSites {
  app: string
  auth: string
  other: string
  /** The second auxiliary port's auth site, for the cases that move between ports of one host. */
  authSecondPort: string
  close(): Promise<void>
}

/** Starts the lab's sites on `ports`: the development page's, and two of the auxiliary sites'. */
export async function startLab(ports: { app: number; aux: number; secondAux: number }): Promise<LabSites> {
  const app = `http://localhost:${ports.app}`
  const auth = `http://auth.localhost:${ports.aux}`
  const other = `http://other.localhost:${ports.aux}`
  const authSecondPort = `http://auth.localhost:${ports.secondAux}`
  // The spike's names for the sites, in the files the lab serves, and the lab's.
  const names: [string, string][] = [
    ['http://localhost:19401', app],
    ['ws://localhost:19401', `ws://localhost:${ports.app}`],
    ['http://127.0.0.1:19401', `http://127.0.0.1:${ports.app}`],
    ['https://auth.upstream.test:19444', auth],
    ['https://other.upstream.test:19444', other],
    ['https://app.upstream.test:19444', `http://app.localhost:${ports.aux}`],
    ['https://auth.upstream.test:19446', authSecondPort],
  ]
  const localize = (text: string) => names.reduce((result, [from, to]) => result.replaceAll(from, to), text)
  const state = { activeStreams: 0, writes: new Map() }
  const echo = new WebSocketServer({ noServer: true })
  echo.on('connection', (socket) => {
    socket.on('message', (data, binary) => socket.send(data, { binary }))
  })
  const handle = async (request: http.IncomingMessage, response: http.ServerResponse) => {
    const url = new URL(request.url ?? '/', `http://${request.headers.host}`)
    response.setHeader('Cache-Control', 'no-store')
    if (await handleLab(request, response, url, state, { appOrigin: app, authOrigin: auth, otherOrigin: other }, localize)) {
      return
    }
    if (url.pathname === '/public-asset.txt') {
      response.writeHead(200, { 'content-type': 'text/plain' })
      response.end('cached-asset-v1')
      return
    }
    response.writeHead(404)
    response.end('Fixture route not found')
  }
  const servers = [ports.app, ports.aux, ports.secondAux].map((port) => {
    const server = http.createServer((request, response) => {
      // A route that fails ends its own exchange, as a site that breaks does.
      handle(request, response).catch((error) => response.destroy(error))
    })
    server.on('upgrade', (request, socket, head) => {
      const path = new URL(request.url ?? '/', 'http://lab').pathname
      if (path === '/lab/reject-ws') {
        socket.end('HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n')
      } else {
        echo.handleUpgrade(request, socket, head, (ws) => echo.emit('connection', ws, request))
      }
    })
    return { server, port }
  })
  for (const { server, port } of servers) {
    await new Promise<void>((resolve, reject) => {
      server.once('error', reject)
      // Every name the lab uses resolves to loopback, IPv6 first on macOS.
      server.listen(port, '::', () => resolve())
    })
  }
  return {
    app,
    auth,
    other,
    authSecondPort,
    async close() {
      for (const client of echo.clients) {
        client.terminate()
      }
      echo.close()
      for (const { server } of servers) {
        server.closeAllConnections()
        await new Promise((resolve) => server.close(resolve))
      }
    },
  }
}
