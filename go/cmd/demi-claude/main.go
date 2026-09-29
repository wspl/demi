// Command demi-claude is the demi.claude command package: it serves the
// operations claude.ensure and claude.status as a command service over its
// standard input and output, which a runner starts with --command-service.
package main

import "github.com/wspl/demi/go/claude"

func main() {
	claude.Main()
}
