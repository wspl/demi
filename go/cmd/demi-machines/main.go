//go:build linux

// Command demi-machines is the Cloud machine manager: it runs users' Cloud
// machines as gVisor sandboxes and serves the backend over a Unix socket
// (docs/cloud/managed-hosts.md).
package main

import (
	"os"

	"github.com/wspl/demi/go/machines"
)

func main() {
	os.Exit(machines.Main())
}
