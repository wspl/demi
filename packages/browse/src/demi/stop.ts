// `demi.stop()`: closes the slot's browser and ends the tool's server once
// the call has answered; the slot's servers keep running until `demi.down`.
import type { Tool } from '../tool'

export async function stop(tool: Tool): Promise<void> {
  await tool.browser.close()
  tool.endServer()
  tool.print('Closed the browser')
}
