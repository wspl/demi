//go:build !faultinjection

// Package fault holds the crash points of acceptance
// (docs/cloud/managed-hosts.md § Verification): a manager built with the
// faultinjection tag aborts at the point DEMI_MACHINES_FAULT names, and the next
// start must recover with nothing left behind. Other builds carry none of it.
package fault

// Point aborts the process here when this point is the injected fault; in this
// build it does nothing.
func Point(name string) {}
