package core

// The latest report of one account.
//
//demi:wire
type QuotaSnapshot struct {
	ObservedAt Timestamp `json:"observedAt" check:"func=Validate"`
	// Which path filled the snapshot last.
	Source       SnapshotSource `json:"source"`
	Plan         *QuotaPlan     `json:"plan" check:"nullable"`
	AccountLabel *string        `json:"accountLabel" check:"nullable"`
	// Each window as its own source said last; one the vendor does not
	// report is absent.
	Windows []QuotaWindow `json:"windows"`
}

// The account's plan: the vendor's id and the name the user reads.
//
//demi:wire
type QuotaPlan struct {
	ID    string `json:"id" check:"chars=1.."`
	Label string `json:"label"`
}

// One limit the vendor meters.
//
//demi:wire
type QuotaWindow struct {
	// A stable id, such as `primary`, `five_hour` or `monthly`.
	ID string `json:"id" check:"chars=1.."`
	// The name the user knows the window by, such as `5-hour`.
	Label string `json:"label"`
	// The share used, from 0 to 100; null when unknown.
	UsedPercent *float64 `json:"usedPercent" check:"nullable,range=0.0..100.0"`
	// The amount used, in the window's unit, when the vendor reports it.
	Used     *float64       `json:"used" check:"nullable"`
	Limit    *float64       `json:"limit" check:"nullable"`
	Unit     *QuotaUnit     `json:"unit" check:"nullable"`
	ResetsAt *Timestamp     `json:"resetsAt" check:"nullable,func=Validate"`
	Severity *QuotaSeverity `json:"severity" check:"nullable"`
	// What the window applies to when that is narrower than the account,
	// such as one model.
	Scope *QuotaScope `json:"scope" check:"nullable"`
}

// The part of an account a window meters, such as one model.
//
//demi:wire
type QuotaScope struct {
	Kind  string  `json:"kind"`
	Label *string `json:"label" check:"nullable"`
}

// How a snapshot was filled.
//
//demi:enum
//demi:export
type SnapshotSource string

const (
	SnapshotSourceProbe       SnapshotSource = "probe"
	SnapshotSourceObservation SnapshotSource = "observation"
)

// The unit of a window's amounts.
//
//demi:enum
//demi:export
type QuotaUnit string

const (
	QuotaUnitPercent  QuotaUnit = "percent"
	QuotaUnitCredits  QuotaUnit = "credits"
	QuotaUnitRequests QuotaUnit = "requests"
	QuotaUnitTokens   QuotaUnit = "tokens"
	QuotaUnitUSDMinor QuotaUnit = "usd_minor"
)

// How close a window is to its limit.
//
//demi:enum
//demi:export
type QuotaSeverity string

const (
	QuotaSeverityNormal   QuotaSeverity = "normal"
	QuotaSeverityWarning  QuotaSeverity = "warning"
	QuotaSeverityCritical QuotaSeverity = "critical"
)
