package core

//demi:wire
type QuotaSnapshot struct {
	ObservedAt   Timestamp      `json:"observedAt" check:"func=Validate"`
	Source       SnapshotSource `json:"source"`
	Plan         *QuotaPlan     `json:"plan" check:"nullable"`
	AccountLabel *string        `json:"accountLabel" check:"nullable"`
	Windows      []QuotaWindow  `json:"windows"`
}

//demi:wire
type QuotaPlan struct {
	ID    string `json:"id" check:"chars=1.."`
	Label string `json:"label"`
}

//demi:wire
type QuotaWindow struct {
	ID          string         `json:"id" check:"chars=1.."`
	Label       string         `json:"label"`
	UsedPercent *float64       `json:"usedPercent" check:"nullable,range=0.0..100.0"`
	Used        *float64       `json:"used" check:"nullable"`
	Limit       *float64       `json:"limit" check:"nullable"`
	Unit        *QuotaUnit     `json:"unit" check:"nullable"`
	ResetsAt    *Timestamp     `json:"resetsAt" check:"nullable,func=Validate"`
	Severity    *QuotaSeverity `json:"severity" check:"nullable"`
	Scope       *QuotaScope    `json:"scope" check:"nullable"`
}

//demi:wire
type QuotaScope struct {
	Kind  string  `json:"kind"`
	Label *string `json:"label" check:"nullable"`
}

//demi:enum
type SnapshotSource string

const (
	SnapshotSourceProbe       SnapshotSource = "probe"
	SnapshotSourceObservation SnapshotSource = "observation"
)

//demi:enum
type QuotaUnit string

const (
	QuotaUnitPercent  QuotaUnit = "percent"
	QuotaUnitCredits  QuotaUnit = "credits"
	QuotaUnitUSDMinor QuotaUnit = "usd_minor"
	QuotaUnitRequests QuotaUnit = "requests"
	QuotaUnitTokens   QuotaUnit = "tokens"
)

//demi:enum
type QuotaSeverity string

const (
	QuotaSeverityNormal   QuotaSeverity = "normal"
	QuotaSeverityWarning  QuotaSeverity = "warning"
	QuotaSeverityCritical QuotaSeverity = "critical"
)
