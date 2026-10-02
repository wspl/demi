// Command demi-claude-code is awaiting migration to Go.
package main

import (
	"fmt"
	"os"
)

func main() {
	// A failed diagnostic write cannot change the required failure exit status.
	_, _ = fmt.Fprintln(os.Stderr, "demi-claude-code: not migrated yet")
	os.Exit(1)
}
