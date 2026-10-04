package cloud

import "sync"

// Capacity holds the permits of every user's Cloud. Share its pointer; its
// operations are safe for concurrent calls. Construct it with NewCapacity.
type Capacity struct {
	mu        sync.Mutex // Protects the number of places available.
	available int
}

// Permit is one Cloud's place among the machines that are not stopped.
// Its owner defers Release or transfers ownership to the machine's next phase.
type Permit struct {
	once     sync.Once
	capacity *Capacity
}

// NewCapacity creates capacity for machines Clouds that are not stopped.
// The caller supplies a nonnegative count from validated lifecycle settings.
func NewCapacity(machines int) *Capacity {
	return &Capacity{available: machines}
}

// TryTake returns a permit now, or nil while every one is taken. It never queues.
func (c *Capacity) TryTake() *Permit {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.available == 0 {
		return nil
	}
	c.available--
	return &Permit{capacity: c}
}

// Release gives the place back. It is idempotent and does not wait.
func (p *Permit) Release() {
	p.once.Do(func() {
		p.capacity.mu.Lock()
		p.capacity.available++
		p.capacity.mu.Unlock()
	})
}
