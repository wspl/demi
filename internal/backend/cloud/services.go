package cloud

// Services is what every shard's Cloud shares: the machine manager's client,
// the capacity across users, and the Cloud's settings. Fields are immutable
// after publication; their pointed-to services synchronize their own state.
type Services struct {
	// Machines is the shared manager connection owner.
	Machines *Client
	// Capacity counts machines across all users.
	Capacity *Capacity
	// Tuning supplies lifecycle times and policy limits.
	Tuning CloudTuning
}

// NewServices creates shared services with tuning.Capacity permits. The caller
// owns machines and closes it after every shard has closed its Cloud.
func NewServices(machines *Client, tuning CloudTuning) *Services {
	return &Services{Machines: machines, Capacity: NewCapacity(tuning.Capacity), Tuning: tuning}
}
