package webapi

// `GET /usage`: the caller's totals.
// +demi:root direction=receive output=web
// +demi:tolerant
type UsageTotals struct {
	Totals []UsageGroup `json:"totals"`
}

// The requests of one provider entry and model.
// +demi:tolerant
type UsageGroup struct {
	ProviderID string `json:"providerId"`
	ModelID    string `json:"modelId"`
	// +demi:range max=9007199254740991
	Requests uint64 `json:"requests"`
	// +demi:range max=9007199254740991
	InputTokens uint64 `json:"inputTokens"`
	// +demi:range max=9007199254740991
	OutputTokens uint64 `json:"outputTokens"`
	// +demi:range max=9007199254740991
	CacheReadTokens uint64 `json:"cacheReadTokens"`
	// +demi:range max=9007199254740991
	CacheWriteTokens uint64 `json:"cacheWriteTokens"`
}

// `GET /usage/instance`: on a shared instance, every account's totals, for
// an administrator.
// +demi:root direction=receive output=web
// +demi:tolerant
type InstanceUsage struct {
	Users []UserUsage `json:"users"`
}

// One account's totals, the accounts in the order they were created.
// +demi:tolerant
type UserUsage struct {
	UserID UserID       `json:"userId"`
	Email  EmailAddress `json:"email"`
	Totals []UsageGroup `json:"totals"`
}
