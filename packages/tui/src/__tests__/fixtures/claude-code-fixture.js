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
    write({
      type: 'control_request',
      request_id: 'outer-tool-1',
      request: {
        subtype: 'mcp_message',
        server_name: 'main',
        message: {
          jsonrpc: '2.0',
          id: 'tool-1',
          method: 'tools/call',
          params: {
            name: 'shell_exec',
            arguments: { script: 'printf fixture-shell' },
          },
        },
      },
    })
    return
  }

  if (message.type === 'control_response' && message.response?.request_id === 'outer-tool-1' && !sentFinal) {
    sentFinal = true
    write({
      type: 'assistant',
      message: {
        content: [
          { type: 'thinking', thinking: 'fixture plan', signature: 'fixture-signature' },
          { type: 'text', text: 'fixture response' },
        ],
      },
    })
    write({
      type: 'result',
      usage: {
        input_tokens: 12,
        output_tokens: 3,
        cache_read_input_tokens: 2,
        cache_creation_input_tokens: 1,
      },
    })
    setTimeout(() => process.exit(0), 10)
  }
})

input.on('close', () => setTimeout(() => process.exit(0), 10))

function write(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`)
}
