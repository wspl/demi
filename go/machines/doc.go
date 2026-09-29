// Package machines is the Cloud machine manager (docs/cloud/managed-hosts.md): it
// runs users' Cloud machines as gVisor sandboxes and serves the backend over a
// Unix socket, in the messages of the machinesproto package. It runs only on
// Linux.
//
// One worker goroutine per device runs that device's operations one at a time, in
// arrival order, and takes its sandbox's exit between two of them. A manager-wide
// admission gate lets reconcile and shutdown wait for the operations in flight.
// Work inside another namespace runs on a thread of its own that ends with the
// job (internal/linux).
package machines

//go:generate go run github.com/wspl/demi/go/cmd/wiregen
