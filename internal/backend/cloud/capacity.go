package cloud

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

// Capacity holds the permits of every user's Cloud. Share its pointer; its
// operations are safe for concurrent calls. Construct it with NewCapacity.
type Capacity struct{}

// Permit is one Cloud's place among the machines that are not stopped.
// Its owner defers Release or transfers ownership to the machine's next phase.
type Permit struct{}

// NewCapacity creates capacity for machines Clouds that are not stopped.
func NewCapacity(machines int) *Capacity { panic("not written: b-cloud") }

// TryTake returns a permit now, or nil while every one is taken. It never queues.
func (c *Capacity) TryTake() *Permit { panic("not written: b-cloud") }

// Release gives the place back. It is idempotent and does not wait.
func (p *Permit) Release() { panic("not written: b-cloud") }
