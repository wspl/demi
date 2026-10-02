// Command demi-backend is awaiting migration to Go.
package main

import (
	"fmt"
	"os"
)

func main() {
	// A failed diagnostic write cannot change the required failure exit status.
	_, _ = fmt.Fprintln(os.Stderr, "demi-backend: not migrated yet")
	os.Exit(1)
}
