package webapi

// `GET /usage`: the caller's totals.
//
//demi:wire open
type UsageTotals struct {
	Totals []UsageGroup `json:"totals"`
}

// The requests of one provider entry and model.
//
//demi:wire open
type UsageGroup struct {
	ProviderID       string `json:"providerId"`
	ModelID          string `json:"modelId"`
	Requests         uint64 `json:"requests" check:"range=..9007199254740991"`
	InputTokens      uint64 `json:"inputTokens" check:"range=..9007199254740991"`
	OutputTokens     uint64 `json:"outputTokens" check:"range=..9007199254740991"`
	CacheReadTokens  uint64 `json:"cacheReadTokens" check:"range=..9007199254740991"`
	CacheWriteTokens uint64 `json:"cacheWriteTokens" check:"range=..9007199254740991"`
}

// `GET /usage/instance`: on a shared instance, every account's totals, for
// an administrator.
//
//demi:wire open
type InstanceUsage struct {
	Users []UserUsage `json:"users"`
}

// One account's totals, the accounts in the order they were created.
//
//demi:wire open
type UserUsage struct {
	UserID UserID       `json:"userId" check:"func=Validate"`
	Email  EmailAddress `json:"email" check:"func=Validate"`
	Totals []UsageGroup `json:"totals"`
}
