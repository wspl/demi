package backend

import (
	"sync"

	"golang.org/x/sync/semaphore"
)

// CloudCapacity is the one Cloud fact that spans users (managed-hosts.md §
// Lifecycle and capacity): how many machines are not stopped. A Cloud takes
// a permit when it leaves the stopped state, to boot or to reset, and gives
// it back only when it is stopped again; a Cloud that finds none free fails
// to start, and nothing queues for one. It is safe for use by any
// goroutine.
type CloudCapacity struct{ permits *semaphore.Weighted }

// CapacityPermit is one Cloud's place among the machines that are not
// stopped.
type CapacityPermit struct{ release func() }

// NewCloudCapacity has room for machines Clouds that are not stopped.
func NewCloudCapacity(machines int64) *CloudCapacity {
	return &CloudCapacity{semaphore.NewWeighted(machines)}
}

// TryTake is a permit, now, or false while every one is taken.
func (c *CloudCapacity) TryTake() (*CapacityPermit, bool) {
	if !c.permits.TryAcquire(1) {
		return nil, false
	}
	return &CapacityPermit{sync.OnceFunc(func() { c.permits.Release(1) })}, true
}

// Release gives the place back; releasing twice gives it back once.
func (p *CapacityPermit) Release() { p.release() }
