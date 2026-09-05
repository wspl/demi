// The primitive conformance suite: the definition of done for tinyjs. Runs as
// the embedded bundle so that it sees tinyjs:*.
import { run } from "./harness.mjs";
import { exit } from "tinyjs:runtime";
import "./globals.mjs";
import "./timers.mjs";
import "./bytes.mjs";
import "./fs.mjs";
import "./runtime.mjs";
import "./process.mjs";
import "./net.mjs";
import "./protocol.mjs";
import "./loader.mjs";

exit(await run());
