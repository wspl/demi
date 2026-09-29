package backend

import (
	"context"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// Health is the provider's authentication and runtime state; made is the
// entry's provider, or buildErr why it could not be built. A subscription
// entry without an account is unauthenticated whatever its provider says.
func (a *ProviderAssembly) Health(ctx context.Context, entry ProviderEntry, made provider.Provider, buildErr error) (core.AuthState, core.RuntimeState) {
	if buildErr != nil {
		return core.AuthStateError{Message: buildErr.Error()}, core.RuntimeStateUnknown{}
	}
	if _, subscription := entry.Credential.(Subscription); subscription && entry.Active() == nil {
		return core.AuthStateUnauthenticated{Message: new("No subscription account configured")}, made.RuntimeState()
	}
	return made.AuthStatus(ctx), made.RuntimeState()
}

// Details are what the product shows of the entry's provider (web-api.md §
// Model configuration and provider inspection): its health, its accounts
// with the quota kept for each, and its capabilities, read when the backend
// answers and never stored. disclose is for a user who configures the
// entry; without it the details hold only what a user who infers may know
// (providers.md § Scope).
func (a *ProviderAssembly) Details(ctx context.Context, entry ProviderEntry, disclose bool) (webapi.ProviderDetails, error) {
	made, err := a.ProviderFor(ctx, entry)
	if err != nil {
		return webapi.ProviderDetails{}, err
	}
	auth, runtime := a.Health(ctx, entry, made, nil)
	details := webapi.ProviderDetails{
		Auth:                       auth,
		Runtime:                    runtime,
		Accounts:                   []webapi.AccountDTO{},
		Active:                     entry.Active(),
		QuotaCapability:            webapi.QuotaCapabilityNone{},
		RequiresProcessCapableHost: made.Capabilities().ProcessHost,
	}
	if _, subscription := entry.Credential.(Subscription); subscription {
		rows, err := a.vault.Accounts(ctx, entry.ID)
		if err != nil {
			return webapi.ProviderDetails{}, err
		}
		for _, row := range rows {
			account := webapi.AccountDTO{AccountInfo: accountMeta(row).Info(), Quota: a.quotas.Latest(entry.ID, row)}
			details.Accounts = append(details.Accounts, account)
			if details.Active != nil && row.ID == *details.Active {
				details.Quota = account.Quota
			}
		}
	}
	if quota, ok := made.(provider.QuotaProvider); ok && quota.Quota() != nil {
		supported := webapi.QuotaCapabilitySupported{}
		if cost := quota.Quota().ProbeCost(); cost != nil {
			supported.Probe = new(probeCostDTO(*cost))
		}
		details.QuotaCapability = supported
	}
	if !disclose {
		return forInferenceOnly(details), nil
	}
	return details, nil
}

// probeCostDTO is what a probe costs, as the browser reads it.
func probeCostDTO(cost provider.ProbeCost) webapi.ProbeCost {
	if cost == provider.ProbeInference {
		return webapi.ProbeCostInference
	}
	return webapi.ProbeCostFree
}

// forInferenceOnly is what a user who only infers may know: no accounts, no
// active account, no quota, and no account label.
func forInferenceOnly(details webapi.ProviderDetails) webapi.ProviderDetails {
	if _, ok := details.Auth.(core.AuthStateAuthenticated); ok {
		details.Auth = core.AuthStateAuthenticated{}
	}
	details.Accounts = []webapi.AccountDTO{}
	details.Active = nil
	details.Quota = nil
	return details
}

// availability says whether the entry's models can be used now, from its
// health: a credential that is missing or refused, or a provider that
// cannot run, makes them unavailable.
func availability(auth core.AuthState, runtime core.RuntimeState) webapi.Availability {
	switch auth.(type) {
	case core.AuthStateUnauthenticated, core.AuthStateError:
		return webapi.AvailabilityUnavailable{Reason: webapi.UnavailableReasonAuthentication, Message: "Provider login is unavailable"}
	}
	switch state := runtime.(type) {
	case core.RuntimeStateUnavailable:
		return webapi.AvailabilityUnavailable{Reason: webapi.UnavailableReasonRuntime, Message: state.Message}
	case core.RuntimeStateError:
		return webapi.AvailabilityUnavailable{Reason: webapi.UnavailableReasonRuntime, Message: state.Message}
	}
	return webapi.AvailabilityAvailable{}
}
