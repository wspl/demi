package core

// QuotaSnapshot holds the latest quota report of an account.
// +demi:root direction=receive output=protocol
type QuotaSnapshot struct {
	ObservedAt Timestamp      `json:"observedAt"`
	Source     SnapshotSource `json:"source"`
	// +demi:nullable
	Plan *QuotaPlan `json:"plan"`
	// +demi:nullable
	AccountLabel *string       `json:"accountLabel"`
	Windows      []QuotaWindow `json:"windows"`
}

// QuotaPlan names the subscription plan reported by a vendor.
type QuotaPlan struct {
	// +demi:length chars min=1
	ID    string `json:"id"`
	Label string `json:"label"`
}

// QuotaWindow describes one metered limit.
type QuotaWindow struct {
	// +demi:length chars min=1
	ID    string `json:"id"`
	Label string `json:"label"`
	// +demi:nullable
	// +demi:range min=0 max=100
	UsedPercent *float64 `json:"usedPercent"`
	// +demi:nullable
	Used *float64 `json:"used"`
	// +demi:nullable
	Limit *float64 `json:"limit"`
	// +demi:nullable
	Unit *QuotaUnit `json:"unit"`
	// +demi:nullable
	ResetsAt *Timestamp `json:"resetsAt"`
	// +demi:nullable
	Severity *QuotaSeverity `json:"severity"`
	// +demi:nullable
	Scope *QuotaScope `json:"scope"`
}

// QuotaScope identifies the part of an account a quota window meters.
type QuotaScope struct {
	Kind string `json:"kind"`
	// +demi:nullable
	Label *string `json:"label"`
}
