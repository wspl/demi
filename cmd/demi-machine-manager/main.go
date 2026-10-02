//go:build !linux

// Command demi-machine-manager runs the privileged Cloud service.
package main

import (
	"fmt"
	"os"
)

func main() {
	fmt.Fprintln(os.Stderr, "demi-machine-manager: the Cloud manager runs only on Linux")
	os.Exit(1)
}
