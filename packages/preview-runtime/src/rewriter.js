// The Rust rewriter (`preview-rewrite`), loaded as WebAssembly: the same code the engine runs on
// the Host, for what the page creates itself. The bundle embeds the module's bytes, and the
// runtime compiles them synchronously, before any page script can run code that needs
// rewriting. `bun xtask preview-runtime` generates both files from `preview-rewrite-wasm`.
import * as wasm from './generated/preview_rewrite_wasm.js';
import bytes from './generated/preview_rewrite_wasm_bg.wasm';

wasm.initSync({ module: new WebAssembly.Module(bytes) });

export { wasm };
