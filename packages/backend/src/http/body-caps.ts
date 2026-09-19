import type { MiddlewareHandler } from 'hono'
import { bodyLimit } from 'hono/body-limit'
import { ATTACHMENT_MAX_BYTES } from './attachments'

/** The most of a JSON body the backend reads whole (`web-api.md` § Request bodies). */
export const JSON_BODY_MAX_BYTES = 1024 * 1024

/** The routes whose body streams to a Host and is never read whole: a runner's pipe, a file upload. */
const STREAMED_BODIES = [/^\/api\/pipes\/[^/]+$/, /^\/api\/conversations\/[^/]+\/fs\/raw$/]

function capped(maxSize: number): MiddlewareHandler {
  return bodyLimit({
    maxSize,
    onError: (c) => c.json({ code: 'too_large', message: `The request body is over its ${maxSize}-byte limit` }, 413),
  })
}

/**
 * Caps every request body the backend reads whole (`web-api.md` § Request
 * bodies): an attachment's at its limit, any other at the JSON limit, each
 * refused before more than that is read. A body a route streams to a Host
 * passes uncapped. The server caps nothing itself, so these are the limits.
 */
export function requestBodyCaps(): MiddlewareHandler {
  const json = capped(JSON_BODY_MAX_BYTES)
  const attachment = capped(ATTACHMENT_MAX_BYTES)
  return (c, next) => {
    const path = c.req.path
    if (c.req.method === 'PUT' && STREAMED_BODIES.some((route) => route.test(path)))
      return next()
    return (/^\/api\/attachments\/?$/.test(path) ? attachment : json)(c, next)
  }
}
