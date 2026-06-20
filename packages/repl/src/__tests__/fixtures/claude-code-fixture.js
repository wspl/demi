import { createInterface } from 'node:readline'

if (process.argv.includes('--version')) {
  console.log('fixture claude 0.0.0')
  process.exit(0)
}

if (process.argv[2] === 'auth' && process.argv[3] === 'status' && process.argv.includes('--json')) {
  console.log(JSON.stringify({ loggedIn: true, email: 'fixture@example.test' }))
  process.exit(0)
}

let sentToolCall = false
let sentFinal = false
let mode = 'basic'

const input = createInterface({ input: process.stdin })
input.on('line', (line) => {
  if (!line.trim()) return
  const message = JSON.parse(line)

  if (message.type === 'control_request' && message.request?.subtype === 'initialize') {
    write({
      type: 'control_response',
      response: {
        subtype: 'success',
        request_id: message.request_id,
        response: {},
      },
    })
    return
  }

  if (message.type === 'user' && !sentToolCall) {
    sentToolCall = true
    const text = userText(message)
    mode = text.includes('interactive input fixture workflow')
      ? 'input'
      : text.includes('flood output fixture workflow')
        ? 'flood'
        : 'basic'
    write({
      type: 'control_request',
      request_id: mode === 'input' ? 'outer-input-tool-1' : mode === 'flood' ? 'outer-flood-tool-1' : 'outer-tool-1',
      request: {
        subtype: 'mcp_message',
        server_name: 'main',
        message: {
          jsonrpc: '2.0',
          id: 'tool-1',
          method: 'tools/call',
          params: {
            name: 'shell_exec',
            arguments:
              mode === 'input'
                ? { script: 'sh -c \'IFS= read -r line; printf "fixture-input:%s" "$line"\'', yieldAfterMs: 1 }
                : mode === 'flood'
                  ? { script: floodScript(), yieldAfterMs: 10_000, outputLimitBytes: 512 * 1024 }
                : { script: 'printf fixture-shell' },
          },
        },
      },
    })
    return
  }

  if (message.type === 'control_response' && message.response?.request_id === 'outer-tool-1' && !sentFinal) {
    sentFinal = true
    writeFinal('fixture response', { input_tokens: 12, output_tokens: 3, cache_read_input_tokens: 2, cache_creation_input_tokens: 1 })
    setTimeout(() => process.exit(0), 10)
  }

  if (message.type === 'control_response' && message.response?.request_id === 'outer-flood-tool-1' && !sentFinal) {
    sentFinal = true
    writeFinal('fixture flood complete', { input_tokens: 14, output_tokens: 5, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 })
    setTimeout(() => process.exit(0), 10)
  }

  if (message.type === 'control_response' && message.response?.request_id === 'outer-input-tool-1' && !sentFinal) {
    sentFinal = true
    writeFinal('fixture input ready', { input_tokens: 13, output_tokens: 4, cache_read_input_tokens: 0, cache_creation_input_tokens: 1 })
    setTimeout(() => process.exit(0), 10)
  }
})

input.on('close', () => setTimeout(() => process.exit(0), 10))

function write(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`)
}

function writeFinal(text, usage) {
  write({
    type: 'assistant',
    message: {
      content: [
        { type: 'thinking', thinking: 'fixture plan', signature: 'fixture-signature' },
        { type: 'text', text },
      ],
    },
  })
  write({ type: 'result', usage })
}

function userText(message) {
  if (!Array.isArray(message.message?.content)) return ''
  return message.message.content.map((block) => (block?.type === 'text' ? String(block.text ?? '') : '')).join('\n')
}

function floodScript() {
  return (
    "awk 'BEGIN { print \"DEMI_FLOOD_START\"; " +
    'for (i = 0; i < 1500; i++) printf "flood-%04d abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789\\n", i; ' +
    'print "DEMI_FLOOD_END" }' +
    "'"
  )
}
