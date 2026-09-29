//go:build linux

package machines

// FenceAndSave is the recovery a manager runs at startup, on reconcile and in the
// stop-post command, for the tests that plant what a crash leaves.
var FenceAndSave = fenceAndSave
