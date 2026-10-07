// The Cloudflare Worker: the preview domain service over the bindings
// `wrangler.toml` declares.
import { createService } from './service'

interface Env {
  /** The preview domain with its port, if any. */
  PREVIEW_DOMAIN: string
  NAMESPACES: KVNamespace
  ASSETS: Fetcher
  NAMESPACE_CREATION: RateLimit
}

export default {
  fetch(request, env) {
    const service = createService({
      domain: env.PREVIEW_DOMAIN,
      store: env.NAMESPACES,
      files: env.ASSETS,
      creationLimiter: env.NAMESPACE_CREATION,
    })
    return service(request)
  },
} satisfies ExportedHandler<Env>
