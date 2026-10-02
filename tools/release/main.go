// Command release packages and publishes Demi's programs, images and records.
package main

import (
	"fmt"
	"os"
)

func main() {
	// A failed diagnostic write cannot change the required failure exit status.
	_, _ = fmt.Fprintln(os.Stderr, "release: not migrated yet")
	os.Exit(1)
}
