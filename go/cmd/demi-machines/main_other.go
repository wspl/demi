//go:build !linux

// Command demi-machines is the Cloud machine manager, which runs only on Linux.
package main

import (
	"fmt"
	"os"
)

func main() {
	fmt.Fprintln(os.Stderr, "demi-machines: the Cloud manager runs only on Linux")
	os.Exit(1)
}
