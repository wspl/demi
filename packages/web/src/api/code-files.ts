import { shallowReactive } from 'vue'
import { apiRequest, jsonBody, readResponse } from './client'
import { existingFilesSchema } from './generated/web-api'

/**
 * The Host's answers about the paths replies' inline code names
 * (`file-previews.md` § Files named in messages), by conversation and the
 * paths of one reply, for the page's lifetime: a reply's answer is kept
 * with it, so it is asked once.
 */
const answers = shallowReactive(new Map<string, ReadonlySet<string>>())

/**
 * The Host's state each question went out under, by its key: a question
 * the Host could not answer, as a stopped Cloud cannot, goes out again only
 * once the Host's state has changed.
 */
const asked = new Map<string, string>()

/**
 * Which of `paths` are files on conversation `conversationId`'s primary
 * Host, whose state is `host`: the answer once it arrived, else null, and
 * the first lookup, or the first after the Host's state changed while it
 * had no answer, asks the backend about them all in one request. A render
 * that read it renders again when the answer arrives.
 */
export function lookupCodeFiles(conversationId: string, paths: readonly string[], host: string): ReadonlySet<string> | null {
  const key = `${conversationId}\n${paths.join('\n')}`
  const answer = answers.get(key)
  if (answer) {
    return answer
  }
  if (asked.get(key) !== host) {
    asked.set(key, host)
    void ask(conversationId, paths, key)
  }
  return null
}

async function ask(conversationId: string, paths: readonly string[], key: string): Promise<void> {
  try {
    const response = await apiRequest(`/conversations/${encodeURIComponent(conversationId)}/fs/exists`, {
      method: 'POST',
      ...jsonBody({ paths }),
    })
    answers.set(key, new Set((await readResponse(response, existingFilesSchema)).files))
  } catch {
    // Unanswered, as while the Cloud is stopped: the code stays code, and
    // the lookup asks again once the Host's state changes.
  }
}
