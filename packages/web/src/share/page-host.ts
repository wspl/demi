import type { PageHost } from '@demicodes/web-ui/plugins/page'
import { BLOCK_PAGES } from '../plugins/generated/blocks'
import { accountlessPageHost } from '../plugins/accountless-host'

/**
 * The plugin pages' host of a shared conversation's page: the pages that draw
 * reply blocks, whose facts the share's pages bring with its texts and whose
 * documents its routes serve for the blocks its text names
 * (`crates/backend-http/src/shares.rs`). Its plugins are on, as their blocks'
 * facts came.
 */
export function sharePageHost(token: string): PageHost {
  return accountlessPageHost('A shared conversation', {
    pages: () => BLOCK_PAGES,
    blockDocument: (plugin, tag, id) =>
      `/api/shares/${encodeURIComponent(token)}/plugins/${encodeURIComponent(plugin)}/blocks/${encodeURIComponent(tag)}/${encodeURIComponent(id)}`,
  })
}
