//revive:disable:unused-parameter API checkpoint retains parameter names for callers; bodies follow after merge.

package cmdpkgs

import "context"

// Decision is a registry retention decision, exposed for deterministic tests.
type Decision uint8

const (
	// Leased means a lease still holds the service; no status was asked.
	Leased Decision = iota
	// Asks means a status request precedes a later decision.
	Asks
	// HoldsConversations means the service holds state and stays resident.
	HoldsConversations
	// Unanswered means status failed and the service stays resident.
	Unanswered
	// Stops means the service holds nothing and stops.
	Stops
)

// DecisionEvent pairs a decision with the executable's digest.
type DecisionEvent struct {
	Digest   string
	Decision Decision
}

// Decisions observes future decisions in order until ctx or the registry ends.
// This test observation stream has the Rust broadcast's 64-event capacity.
func (r *ServiceRegistry) Decisions(ctx context.Context) *DecisionReceiver {
	panic("not written: r-cmdpkgs")
}

// DecisionReceiver observes registry decisions without holding services alive.
type DecisionReceiver struct{}

// Next waits for a decision, reporting lag or closure as an error.
func (r *DecisionReceiver) Next(ctx context.Context) (DecisionEvent, error) {
	panic("not written: r-cmdpkgs")
}
