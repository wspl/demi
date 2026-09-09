export interface HttpResponse {
  status: number
  headers: Record<string, string>
  body: AsyncIterable<Uint8Array>
}

/** Fetch pulls the producer only as the network accepts more bytes. */
export async function httpPut(
  url: string,
  source: AsyncIterable<Uint8Array>,
  headers: Record<string, string>
): Promise<HttpResponse> {
  const reader = source instanceof ReadableStream
    ? source.getReader()
    : undefined
  const iterator = reader ? undefined : source[Symbol.asyncIterator]()
  let finished = false
  const releaseSource = async () => {
    if (finished)
      return
    finished = true
    if (reader) {
      await reader.cancel()
      reader.releaseLock()
    } else {
      await iterator!.return?.()
    }
  }
  const body = new ReadableStream<Uint8Array>({
    async pull(controller) {
      const result = reader ? await reader.read() : await iterator!.next()
      if (finished)
        return
      if (result.done) {
        finished = true
        reader?.releaseLock()
        controller.close()
      } else {
        controller.enqueue(result.value)
      }
    },
    cancel: releaseSource,
  }, { highWaterMark: 0 })
  try {
    return responseOf(await fetch(
      url,
      { method: 'PUT', headers, body, duplex: 'half' }
    ))
  } finally {
    await releaseSource()
  }
}

export async function httpGet(
  url: string,
  headers: Record<string, string>
): Promise<HttpResponse> {
  return responseOf(await fetch(url, { headers }))
}

function responseOf(response: Response): HttpResponse {
  return {
    status: response.status,
    headers: Object.fromEntries(response.headers),
    body: response.body ?? new ReadableStream({
      start(controller) {
        controller.close()
      },
    }),
  }
}
