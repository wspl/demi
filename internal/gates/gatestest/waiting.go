package gatestest

import "github.com/wspl/demi/internal/gates"

// Waiting observes how many entrants are waiting for a lease, together with
// a channel closed when that count changes. Read again after notification.
// In a synctest bubble, call synctest.Wait first to observe settled waiters.
func Waiting(activity *gates.Activity) (int, <-chan struct{}) {
	return activity.TestingWaiting()
}
