let draws: boolean | null = null

/** Whether the browser draws WebGL 2, which three.js needs: asked once, its context let go at once. */
export function drawsWebgl(): boolean {
  if (draws === null) {
    const context = document.createElement('canvas').getContext('webgl2')
    draws = context !== null
    context?.getExtension('WEBGL_lose_context')?.loseContext()
  }
  return draws
}
