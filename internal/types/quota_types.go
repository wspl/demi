package types

// The latest report of one account.
// +demi:root direction=receive output=protocol
type QuotaSnapshot struct {
	ObservedAt Timestamp `json:"observedAt"`
	// Which path filled the snapshot last.
	Source SnapshotSource `json:"source"`
	// +demi:nullable
	Plan *QuotaPlan `json:"plan"`
	// +demi:nullable
	AccountLabel *string `json:"accountLabel"`
	// Each window as its own source said last; one the vendor does not
	// report is absent.
	Windows []QuotaWindow `json:"windows"`
}

// The account's plan: the vendor's id and the name the user reads.
type QuotaPlan struct {
	// +demi:length chars min=1
	ID    string `json:"id"`
	Label string `json:"label"`
}

// One limit the vendor meters.
type QuotaWindow struct {
	// A stable id, such as `primary`, `five_hour` or `monthly`.
	// +demi:length chars min=1
	ID string `json:"id"`
	// The name the user knows the window by, such as `5-hour`.
	Label string `json:"label"`
	// The share used, from 0 to 100; null when unknown.
	// +demi:nullable
	// +demi:range min=0 max=100
	UsedPercent *float64 `json:"usedPercent"`
	// The amount used, in the window's unit, when the vendor reports it.
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
	// What the window applies to when that is narrower than the account,
	// such as one model.
	// +demi:nullable
	Scope *QuotaScope `json:"scope"`
}

// The part of an account a window meters, such as one model.
type QuotaScope struct {
	Kind string `json:"kind"`
	// +demi:nullable
	Label *string `json:"label"`
}
