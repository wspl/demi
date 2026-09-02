// Entry-mode skeleton. The runner (M9) and the command loader (M8) replace
// this file with the bundle built from @demicodes/host-tinyjs and friends.
import { argv, exit } from "tinyjs:runtime";

const name = argv[0].slice(argv[0].lastIndexOf("/") + 1);
const mode = name === "demi-runner" ? "runner" : "command";
console.log(`tinyjs: ${mode} mode (${name}) is not wired yet`);
exit(mode === "runner" ? 2 : 0);
