// Command demi-commands is the demi.builtin command package: it serves the
// file operations (and, with the conversation browser, the browser's) as a
// command service over its standard input and output, which a runner starts
// with --command-service.
package main

import "github.com/wspl/demi/go/builtincommands"

func main() {
	builtincommands.Main()
}
