// Package host defines execution-target, command-handler and shell-environment
// contracts. It also owns command declarations paired with RPC handlers and
// command output records with separate model and page views. Concrete Hosts,
// shell environments and persistence belong to their implementing packages.
// Blocking operations take contexts; callers explicitly close streams and
// process controls and join started processes. The package starts no goroutines.
package host
