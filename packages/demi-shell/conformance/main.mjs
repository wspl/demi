// The primitive conformance suite: the shell's definition of done. Runs as
// the embedded bundle so that it sees demishell:*.
import { run } from "./harness.mjs";
import { exit } from "demishell:runtime";
import "./globals.mjs";
import "./timers.mjs";
import "./bytes.mjs";
import "./fs.mjs";
import "./runtime.mjs";
import "./process.mjs";
import "./loader.mjs";

exit(await run());
