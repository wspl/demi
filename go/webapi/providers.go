package webapi

import (
	"github.com/wspl/demi/go/core"
)

// How an entry authenticates (`providers.md` § Families, vendors and
// endpoints).
//
//demi:enum
//demi:export
type CredentialKind string

const (
	// A key the user types in.
	CredentialKindAPIKey CredentialKind = "api_key"
	// Accounts from a device login or a setup-token import.
	CredentialKindSubscription CredentialKind = "subscription"
)

// One model of an entry's configured list, which states its facts directly
// (`models.md` § Catalog sources): its first thinking effort is its
// default, its Fast tier, when it names one, is its only service tier, and
// it is taken to call tools.
//
//demi:wire
type ConfiguredModel struct {
	ID          Trimmed `json:"id" check:"chars=1..256,func=Validate"`
	DisplayName Trimmed `json:"displayName" check:"chars=1..256,func=Validate"`
	// Tokens.
	ContextWindow uint32 `json:"contextWindow" check:"range=1.."`
	// Null for no model-specific limit; never beyond the context window.
	OutputLimit *uint32 `json:"outputLimit" check:"nullable,range=1.."`
	// The vendor's effort levels; empty for a model without thinking.
	ThinkingEfforts []string `json:"thinkingEfforts" check:"items=..32,each(chars=1..64)"`
	// The types the model reads natively: `[]` for none, null when unknown.
	AcceptedExtensions *[]core.FileExtension `json:"acceptedExtensions" check:"nullable,items=..64,each(func=core.Validate)"`
	// The service tier the model's Fast is; null for none.
	FastTier *string `json:"fastTier" check:"nullable,chars=1..64"`
}

// `POST /providers`: an API-key entry from the vendor list, whose family,
// wire and endpoint the vendor supplies, or a custom endpoint that names its
// family itself.
//
//demi:union tag=source
//demi:export
type CreateProvider interface{ isCreateProvider() }

//demi:variant vendor
type CreateProviderVendor struct {
	VendorID string  `json:"vendorId" check:"bytes=1.."`
	Label    Trimmed `json:"label" check:"chars=1..80,func=Validate"`
	APIKey   string  `json:"apiKey" check:"bytes=1.."`
	// Replaces the vendor's endpoint.
	BaseURL *EndpointURL `json:"baseUrl,omitzero" check:"func=Validate"`
	// Replaces the vendor's live model list.
	Models *ConfiguredModels `json:"models,omitzero" check:"func=Validate"`
}

func (CreateProviderVendor) isCreateProvider() {}

//demi:variant custom
type CreateProviderCustom struct {
	ProviderType string `json:"providerType" check:"bytes=1.."`
	// For the `openai` family only; its default is Responses.
	WireAPI *core.WireAPI     `json:"wireApi,omitzero" check:"func=core.Validate"`
	Label   Trimmed           `json:"label" check:"chars=1..80,func=Validate"`
	APIKey  string            `json:"apiKey" check:"bytes=1.."`
	BaseURL *EndpointURL      `json:"baseUrl,omitzero" check:"func=Validate"`
	Models  *ConfiguredModels `json:"models,omitzero" check:"func=Validate"`
}

func (CreateProviderCustom) isCreateProvider() {}

// `PATCH /providers/:id`: a new label for any entry; for an API-key entry
// also a new key, endpoint and model list, where `null` removes the
// endpoint override or returns to the live model list.
//
//demi:wire
type ProviderPatch struct {
	Label   *Trimmed           `json:"label,omitzero" check:"chars=1..80,func=Validate"`
	APIKey  *string            `json:"apiKey,omitzero" check:"bytes=1.."`
	BaseURL **EndpointURL      `json:"baseUrl,omitzero" check:"nullable,func=Validate"`
	Models  **ConfiguredModels `json:"models,omitzero" check:"nullable,func=Validate"`
}

// An entry as the browser sees it: its family, label, endpoint, vendor and
// model list, never its key.
//
//demi:wire open
type ProviderDTO struct {
	ID   ProviderID     `json:"id" check:"func=Validate"`
	Kind CredentialKind `json:"kind"`
	// The entry's family, such as `openai` or `codex`.
	ProviderType string `json:"providerType"`
	Label        string `json:"label"`
	// The wire of an `openai` entry that names one; null otherwise.
	WireAPI *core.WireAPI `json:"wireApi" check:"nullable,func=core.Validate"`
	// The vendor an entry was added from; null for a custom endpoint and a
	// subscription.
	VendorID *string `json:"vendorId" check:"nullable"`
	// The configured endpoint; null for the family's default.
	BaseURL *EndpointURL `json:"baseUrl" check:"nullable,func=Validate"`
	// The configured model list; null for the entry's live catalog.
	Models    *ConfiguredModels `json:"models" check:"nullable,func=Validate"`
	CreatedAt core.Timestamp    `json:"createdAt" check:"func=core.Validate"`
}

// `{ provider }`: the answer of a create, an edit and a setup-token import.
//
//demi:wire open
type ProviderAnswer struct {
	Provider ProviderDTO `json:"provider"`
}

// `GET /providers`: the entries of the caller's scope, oldest first.
//
//demi:wire open
type Providers struct {
	Providers []ProviderDTO `json:"providers"`
}

// How the product learns an account's quota.
//
//demi:union tag=type
//demi:export
type QuotaCapability interface{ isQuotaCapability() }

// The family reports no quota.
//
//demi:variant none open
type QuotaCapabilityNone struct {
}

func (QuotaCapabilityNone) isQuotaCapability() {}

// The family reports quota, from responses and, when `probe` names its
// cost, from a probe of its usage endpoint.
//
//demi:variant supported open
type QuotaCapabilitySupported struct {
	Probe *ProbeCost `json:"probe" check:"nullable"`
}

func (QuotaCapabilitySupported) isQuotaCapability() {}

// What a quota probe costs.
//
//demi:enum
//demi:export
type ProbeCost string

const (
	// It reads a usage endpoint.
	ProbeCostFree ProbeCost = "free"
	// It would spend an inference request, which the backend never runs.
	ProbeCostInference ProbeCost = "inference"
)

// An account with the quota snapshot kept for it.
//
//demi:wire open
type AccountDTO struct {
	core.AccountInfo
	// The account's last real snapshot; null before any.
	Quota *core.QuotaSnapshot `json:"quota" check:"nullable,func=core.Validate"`
}

// What `GET /providers/:id/status` answers, read when it answers and never
// stored: the provider's health, its accounts and their quota. For a user
// who only infers with the entry, `accounts` is empty, `active` and `quota`
// are null, and an authenticated `auth` names no account.
//
//demi:wire open
type ProviderDetails struct {
	Auth     core.AuthState    `json:"auth" check:"func=core.Validate"`
	Runtime  core.RuntimeState `json:"runtime" check:"func=core.Validate"`
	Accounts []AccountDTO      `json:"accounts"`
	// The account the entry infers with.
	Active *CredentialID `json:"active" check:"nullable,func=Validate"`
	// The active account's snapshot.
	Quota           *core.QuotaSnapshot `json:"quota" check:"nullable,func=core.Validate"`
	QuotaCapability QuotaCapability     `json:"quotaCapability"`
	// Whether the provider runs a process on the user's Cloud.
	RequiresProcessCapableHost bool `json:"requiresProcessCapableHost"`
}

// An entry in the product state: the entry and what the backend could read
// of its provider.
//
//demi:wire open
type ProviderState struct {
	ProviderDTO
	Details ProviderReading `json:"details"`
}

// What the backend read of an entry's provider when it answered.
//
//demi:union tag=type
//demi:export
type ProviderReading interface{ isProviderReading() }

//demi:variant read open
//wiregen:browser extends ProviderDetails
type ProviderReadingRead struct {
	ProviderDetails
}

func (ProviderReadingRead) isProviderReading() {}

// The provider could not be built or read; the other entries are
// unaffected.
//
//demi:variant failed open
type ProviderReadingFailed struct {
	Message string `json:"message"`
}

func (ProviderReadingFailed) isProviderReading() {}

// `POST /providers/:id/quota`: the account to probe; the active one
// without it.
//
//demi:wire
type QuotaRequest struct {
	CredentialID *CredentialID `json:"credentialId,omitzero" check:"func=Validate"`
}

// `{ quota }`: the account's snapshot after the probe, or the kept one of a
// family that cannot probe; null when there is none.
//
//demi:wire open
type QuotaAnswer struct {
	Quota *core.QuotaSnapshot `json:"quota" check:"nullable,func=core.Validate"`
}

// `POST /providers/:id/test`: one real request to the model, with the
// account the caller names or the active one.
//
//demi:wire
type TestRequest struct {
	ModelID      string        `json:"modelId" check:"bytes=1.."`
	CredentialID *CredentialID `json:"credentialId,omitzero" check:"func=Validate"`
}

// What a test found. A test that ran and failed is a result, with the
// provider's own reason.
//
//demi:union tag=type
//demi:export
type TestResult interface{ isTestResult() }

// The model answered; `model` is its display name.
//
//demi:variant passed open
type TestResultPassed struct {
	Model string `json:"model"`
}

func (TestResultPassed) isTestResult() {}

//demi:variant failed open
type TestResultFailed struct {
	Message string `json:"message"`
	// The model's display name, when the catalog lists it.
	Model *string `json:"model,omitzero"`
}

func (TestResultFailed) isTestResult() {}

// `GET /providers/catalog`: what the page can add.
//
//demi:wire open
type VendorCatalog struct {
	// Each subscription family, with whether the scope holds its entry.
	Subscriptions []SubscriptionFamily `json:"subscriptions"`
	// The models.dev vendors a family speaks to, by name.
	Vendors []Vendor `json:"vendors"`
}

//demi:wire open
type SubscriptionFamily struct {
	ProviderType string `json:"providerType"`
	Configured   bool   `json:"configured"`
}

// A vendor the page can add an entry from.
//
//demi:wire open
type Vendor struct {
	ID   string `json:"id"`
	Name string `json:"name"`
	// The family that speaks the vendor's protocol.
	ProviderType string        `json:"providerType"`
	WireAPI      *core.WireAPI `json:"wireApi,omitzero" check:"func=core.Validate"`
	// The endpoint an entry starts with.
	BaseURL *string `json:"baseUrl" check:"nullable"`
	Doc     *string `json:"doc" check:"nullable"`
}

// `POST /providers/setup-token`: a Claude Code entry from a token that
// `claude setup-token` printed.
//
//demi:wire
type SetupTokenImport struct {
	Token Trimmed `json:"token" check:"chars=1..16384,func=Validate"`
	Label Trimmed `json:"label" check:"chars=1..80,func=Validate"`
}

// `POST /providers/:id/accounts`: another account of an entry, from a setup
// token.
//
//demi:wire
type AddToken struct {
	Token Trimmed `json:"token" check:"chars=1..16384,func=Validate"`
}

// `{ account }`: the account a token import added.
//
//demi:wire open
type AddedAccount struct {
	Account core.AccountInfo `json:"account" check:"func=core.Validate"`
}

// `GET /providers/:id/accounts`: the entry's accounts and the active one.
//
//demi:wire open
type Accounts struct {
	Accounts []core.AccountInfo `json:"accounts" check:"each(func=core.Validate)"`
	Active   *CredentialID      `json:"active" check:"nullable,func=Validate"`
}

// `PUT /providers/:id/accounts/active`.
//
//demi:wire
type ActivateAccount struct {
	CredentialID CredentialID `json:"credentialId" check:"func=Validate"`
}

// `{ active }`: the account the entry now infers with.
//
//demi:wire open
type ActiveAccount struct {
	Active CredentialID `json:"active" check:"func=Validate"`
}

// `POST /providers/subscription-login`: the first account of a family's
// entry, by device login.
//
//demi:wire
type SubscriptionLogin struct {
	ProviderType string `json:"providerType" check:"bytes=1.."`
	// The new entry's label; the family's subscription name without it.
	Label *Trimmed `json:"label,omitzero" check:"chars=1..80,func=Validate"`
}

// The 202 answer of a login start: `{ login: { id, status: "pending" } }`.
//
//demi:wire open
type LoginStarted struct {
	Login StartedLogin `json:"login"`
}

//demi:wire open
type StartedLogin struct {
	ID     LoginID       `json:"id" check:"func=Validate"`
	Status PendingStatus `json:"status"`
}

// The status of a login that has just started.
//
//demi:enum
//demi:export
type PendingStatus string

const (
	PendingStatusPending PendingStatus = "pending"
)

// `GET /providers/subscription-login/:id`.
//
//demi:wire open
type LoginAnswer struct {
	Login LoginState `json:"login"`
}

// Where a device login is; a finished one is kept for ten minutes.
//
//demi:union tag=status
//demi:export
type LoginState interface{ isLoginState() }

// Waiting for the user, who opens the address and enters the code;
// both are null until the vendor names them.
//
//demi:variant pending open
type LoginStatePending struct {
	VerificationURL *string         `json:"verificationUrl" check:"nullable"`
	UserCode        *string         `json:"userCode" check:"nullable"`
	ExpiresAt       *core.Timestamp `json:"expiresAt" check:"nullable,func=core.Validate"`
}

func (LoginStatePending) isLoginState() {}

// The login stored its account: `credentialId`, which is the entry's
// active account only when the entry had none.
//
//demi:variant completed open
type LoginStateCompleted struct {
	ProviderID   ProviderID   `json:"providerId" check:"func=Validate"`
	CredentialID CredentialID `json:"credentialId" check:"func=Validate"`
}

func (LoginStateCompleted) isLoginState() {}

//demi:variant failed open
type LoginStateFailed struct {
	Message string `json:"message"`
}

func (LoginStateFailed) isLoginState() {}

// `GET /models`: the catalog of every entry the caller infers with.
//
//demi:wire open
type ModelCatalog struct {
	Providers []CatalogProvider `json:"providers"`
}

// One entry's catalog with the provider's health, read when the backend
// answers.
//
//demi:wire open
type CatalogProvider struct {
	ProviderID                 ProviderID     `json:"providerId" check:"func=Validate"`
	DisplayName                string         `json:"displayName"`
	RequiresProcessCapableHost bool           `json:"requiresProcessCapableHost"`
	Models                     []CatalogModel `json:"models"`
	// When the source was last downloaded; the Unix epoch for a configured
	// or built-in list, and for a catalog no refresh has filled.
	SourceFetchedAt core.Timestamp `json:"sourceFetchedAt" check:"func=core.Validate"`
	// Whether this is a copy kept after a failed refresh.
	Stale        bool              `json:"stale"`
	Warnings     []string          `json:"warnings"`
	Auth         core.AuthState    `json:"auth" check:"func=core.Validate"`
	Runtime      core.RuntimeState `json:"runtime" check:"func=core.Validate"`
	Availability Availability      `json:"availability"`
}

// A catalog model with the selection the backend built from it, so the
// browser never converts one itself.
//
//demi:wire open
type CatalogModel struct {
	core.ProviderModel
	Selection core.ModelSelection `json:"selection" check:"func=core.Validate"`
	// The thinking effort a conversation's model settings hold on this
	// model when a change names none: null, no thinking setting, for a
	// model that can turn thinking off (`models.md` § A conversation's
	// model settings).
	UnnamedEffort *string `json:"unnamedEffort" check:"nullable"`
}

// Whether the entry's models can be used now, from its health.
//
//demi:union tag=type
//demi:export
type Availability interface{ isAvailability() }

//demi:variant available open
type AvailabilityAvailable struct {
}

func (AvailabilityAvailable) isAvailability() {}

//demi:variant unavailable open
type AvailabilityUnavailable struct {
	Reason  UnavailableReason `json:"reason"`
	Message string            `json:"message"`
}

func (AvailabilityUnavailable) isAvailability() {}

// What makes an entry's models unavailable.
//
//demi:enum
//demi:export
type UnavailableReason string

const (
	// The credential is missing or refused.
	UnavailableReasonAuthentication UnavailableReason = "authentication"
	// The provider cannot run requests.
	UnavailableReasonRuntime UnavailableReason = "runtime"
)

// `GET /providers/:id/cli`: the command-line tool of an entry whose
// provider runs one on the user's Cloud. Reading it wakes nothing.
//
//demi:wire open
type ProviderCLI struct {
	Newest NewestVersion `json:"newest"`
	// The last install on the caller's Cloud that no conversation asked
	// for, since the backend started; null before one.
	Install *CLIInstall `json:"install" check:"nullable"`
	// The caller's Cloud while its runner is connected; empty otherwise.
	Machines []CLIMachine `json:"machines"`
}

// The vendor's newest version, or why it could not be read.
//
//demi:union tag=type
//demi:export
type NewestVersion interface{ isNewestVersion() }

//demi:variant read open
type NewestVersionRead struct {
	Version string `json:"version"`
}

func (NewestVersionRead) isNewestVersion() {}

//demi:variant unreadable open
type NewestVersionUnreadable struct {
	Message string `json:"message"`
}

func (NewestVersionUnreadable) isNewestVersion() {}

// Where an install of the tool stands.
//
//demi:union tag=state
//demi:export
type CLIInstall interface{ isCLIInstall() }

//demi:variant installing open
type CLIInstallInstalling struct {
}

func (CLIInstallInstalling) isCLIInstall() {}

// `path` is the executable on the Cloud.
//
//demi:variant installed open
type CLIInstallInstalled struct {
	Path string `json:"path"`
}

func (CLIInstallInstalled) isCLIInstall() {}

//demi:variant failed open
type CLIInstallFailed struct {
	Message string `json:"message"`
}

func (CLIInstallFailed) isCLIInstall() {}

// A machine the tool runs on, with the versions it has, newest first;
// null when it did not answer.
//
//demi:wire open
type CLIMachine struct {
	DeviceID DeviceID  `json:"deviceId" check:"func=Validate"`
	Name     string    `json:"name"`
	Versions *[]string `json:"versions" check:"nullable"`
}

// The 202 answer of `POST /providers/:id/cli/install`.
//
//demi:wire open
type CLIInstallAnswer struct {
	Install CLIInstall `json:"install"`
}
