package webapiproto

import (
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/types"
)

// The most characters (Unicode scalar values) a pasted setup token has,
// after trimming.
const TokenMax = 16384

// The most characters (Unicode scalar values) an entry's label has, after
// trimming.
const LabelMax = 80

// How an entry authenticates (`providers.md` § Families, vendors and
// endpoints).
// +demi:enum api_key subscription
type CredentialKind string

// Values of the preceding enumeration.
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
// +demi:check validateConfiguredModel
type ConfiguredModel struct {
	// +demi:length chars min=1 max=256
	ID Trimmed `json:"id"`
	// +demi:length chars min=1 max=256
	DisplayName Trimmed `json:"displayName"`
	// Tokens.
	// +demi:range min=1
	ContextWindow uint32 `json:"contextWindow"`
	// Null for no model-specific limit; never beyond the context window.
	// +demi:range min=1
	// +demi:nullable
	OutputLimit *uint32 `json:"outputLimit"`
	// The vendor's effort levels; empty for a model without thinking.
	// +demi:length max=32
	ThinkingEfforts []ThinkingEffort `json:"thinkingEfforts"`
	// The types the model reads natively: `[]` for none, null when unknown.
	// +demi:length max=64
	// +demi:nullable
	AcceptedExtensions *[]types.FileExtension `json:"acceptedExtensions"`
	// The service tier the model's Fast is; null for none.
	// +demi:length chars min=1 max=64
	// +demi:nullable
	FastTier *string `json:"fastTier"`
}

// `POST /providers`: an API-key entry from the vendor list, whose family,
// wire and endpoint the vendor supplies, or a custom endpoint that names its
// family itself.
// +demi:root direction=send output=web
// +demi:union tag=source
//
//sumtype:decl
type CreateProvider interface{ createProvider() }

// +demi:variant CreateProvider vendor
type CreateProviderVendor struct {
	// +demi:length chars min=1
	VendorID string `json:"vendorId"`
	// +demi:length chars min=1 max=80
	Label Trimmed `json:"label"`
	// +demi:length chars min=1
	APIKey string `json:"apiKey"`
	// Replaces the vendor's endpoint.
	BaseURL *EndpointURL `json:"baseUrl,omitempty"`
	// Replaces the vendor's live model list.
	Models *ConfiguredModels `json:"models,omitempty"`
}

// +demi:variant CreateProvider custom
type CreateProviderCustom struct {
	// +demi:length chars min=1
	ProviderType string `json:"providerType"`
	// For the `openai` family only; its default is Responses.
	WireAPI *types.WireAPI `json:"wireApi,omitempty"`
	// +demi:length chars min=1 max=80
	Label Trimmed `json:"label"`
	// +demi:length chars min=1
	APIKey  string            `json:"apiKey"`
	BaseURL *EndpointURL      `json:"baseUrl,omitempty"`
	Models  *ConfiguredModels `json:"models,omitempty"`
}

// `PATCH /providers/:id`: a new label for any entry; for an API-key entry
// also a new key, endpoint and model list, where `null` removes the
// endpoint override or returns to the live model list.
// +demi:root direction=send output=web
type ProviderPatch struct {
	// +demi:length chars min=1 max=80
	Label *Trimmed `json:"label,omitempty"`
	// +demi:length chars min=1
	APIKey *string `json:"apiKey,omitempty"`
	// +demi:nullable
	BaseURL **EndpointURL `json:"baseUrl,omitempty"`
	// +demi:nullable
	Models **ConfiguredModels `json:"models,omitempty"`
}

// An entry as the web app sees it: its family, label, endpoint, vendor and
// model list, never its key.
// +demi:tolerant
type ProviderDTO struct {
	ID   ProviderID     `json:"id"`
	Kind CredentialKind `json:"kind"`
	// The entry's family, such as `openai` or `codex`.
	ProviderType string `json:"providerType"`
	Label        string `json:"label"`
	// The wire of an `openai` entry that names one; null otherwise.
	// +demi:nullable
	WireAPI *types.WireAPI `json:"wireApi"`
	// The vendor an entry was added from; null for a custom endpoint and a
	// subscription.
	// +demi:nullable
	VendorID *string `json:"vendorId"`
	// The configured endpoint; null for the family's default.
	// +demi:nullable
	BaseURL *EndpointURL `json:"baseUrl"`
	// The configured model list; null for the entry's live catalog.
	// +demi:nullable
	Models    *ConfiguredModels `json:"models"`
	CreatedAt types.Timestamp   `json:"createdAt"`
}

// `{ provider }`: the answer of a create, an edit and a setup-token import.
// +demi:root direction=receive output=web
// +demi:tolerant
type ProviderAnswer struct {
	Provider ProviderDTO `json:"provider"`
}

// `GET /providers`: the entries of the caller's scope, oldest first.
// +demi:root direction=receive output=web
// +demi:tolerant
type Providers struct {
	Providers []ProviderDTO `json:"providers"`
}

// How the product learns an account's quota.
// +demi:union tag=type
//
//sumtype:decl
type QuotaCapability interface{ quotaCapability() }

// The family reports no quota.
// +demi:variant QuotaCapability none
// +demi:tolerant
type QuotaCapabilityNone struct{}

// The family reports quota, from responses and, when `probe` names its
// cost, from a probe of its usage endpoint.
// +demi:variant QuotaCapability supported
// +demi:tolerant
type QuotaCapabilitySupported struct {
	// +demi:nullable
	Probe *ProbeCost `json:"probe"`
}

// What a quota probe costs.
// +demi:enum free inference
type ProbeCost string

// Values of the preceding enumeration.
const (
	// It reads a usage endpoint.
	ProbeCostFree ProbeCost = "free"
	// It would spend an inference request, which the backend never runs.
	ProbeCostInference ProbeCost = "inference"
)

// An account with the quota snapshot kept for it.
// +demi:tolerant
type AccountDTO struct {
	types.AccountInfo
	// The account's last real snapshot; null before any.
	// +demi:nullable
	Quota *types.QuotaSnapshot `json:"quota"`
}

// What `GET /providers/:id/status` answers, read when it answers and never
// stored: the provider's health, its accounts and their quota. For a user
// who only infers with the entry, `accounts` is empty, `active` and `quota`
// are null, and an authenticated `auth` names no account.
// +demi:root direction=receive output=web
// +demi:tolerant
type ProviderDetails struct {
	Auth     types.AuthState    `json:"auth"`
	Runtime  types.RuntimeState `json:"runtime"`
	Accounts []AccountDTO       `json:"accounts"`
	// The account the entry infers with.
	// +demi:nullable
	Active *CredentialID `json:"active"`
	// The active account's snapshot.
	// +demi:nullable
	Quota           *types.QuotaSnapshot `json:"quota"`
	QuotaCapability QuotaCapability      `json:"quotaCapability"`
	// The command package that installs the CLI the provider's requests
	// start on the user's Cloud, such as `demi.claude-code`; null when its
	// requests are HTTP.
	// +demi:nullable
	CLIPackage *string `json:"cliPackage"`
}

// An entry in the product state: the entry and what the backend could read
// of its provider.
// +demi:tolerant
type ProviderState struct {
	ProviderDTO
	Details ProviderReading `json:"details"`
}

// What the backend read of an entry's provider when it answered.
// +demi:union tag=type
//
//sumtype:decl
type ProviderReading interface{ providerReading() }

// +demi:variant ProviderReading read
// +demi:tolerant
type ProviderReadingRead struct {
	ProviderDetails
}

// The provider could not be built or read; the other entries are
// unaffected.
// +demi:variant ProviderReading failed
// +demi:tolerant
type ProviderReadingFailed struct {
	Message string `json:"message"`
}

// `POST /providers/:id/quota`: the account to probe; the active one
// without it.
// +demi:root direction=send output=web
type QuotaRequest struct {
	CredentialID *CredentialID `json:"credentialId,omitempty"`
}

// `{ quota }`: the account's snapshot after the probe, or the kept one of a
// family that cannot probe; null when there is none.
// +demi:root direction=receive output=web
// +demi:tolerant
type QuotaAnswer struct {
	// +demi:nullable
	Quota *types.QuotaSnapshot `json:"quota"`
}

// `POST /providers/:id/test`: one real request to the model, with the
// account the caller names or the active one.
// +demi:root direction=send output=web
type TestRequest struct {
	// +demi:length chars min=1
	ModelID      string        `json:"modelId"`
	CredentialID *CredentialID `json:"credentialId,omitempty"`
}

// What a test found. A test that ran and failed is a result, with the
// provider's own reason.
// +demi:root direction=receive output=web
// +demi:union tag=type
//
//sumtype:decl
type TestResult interface{ testResult() }

// The model answered; `model` is its display name.
// +demi:variant TestResult passed
// +demi:tolerant
type TestResultPassed struct {
	Model string `json:"model"`
}

// +demi:variant TestResult failed
// +demi:tolerant
type TestResultFailed struct {
	Message string `json:"message"`
	// The model's display name, when the catalog lists it.
	Model *string `json:"model,omitempty"`
}

// `GET /providers/catalog`: what the page can add.
// +demi:root direction=receive output=web
// +demi:tolerant
type VendorCatalog struct {
	// Each subscription family, with whether the scope holds its entry.
	Subscriptions []SubscriptionFamily `json:"subscriptions"`
	// The models.dev vendors a family speaks to, by name.
	Vendors []Vendor `json:"vendors"`
}

// +demi:tolerant
type SubscriptionFamily struct {
	ProviderType string `json:"providerType"`
	Configured   bool   `json:"configured"`
}

// A vendor the page can add an entry from.
// +demi:tolerant
type Vendor struct {
	ID   string `json:"id"`
	Name string `json:"name"`
	// The family that speaks the vendor's protocol.
	ProviderType string         `json:"providerType"`
	WireAPI      *types.WireAPI `json:"wireApi,omitempty"`
	// The endpoint an entry starts with.
	// +demi:nullable
	BaseURL *string `json:"baseUrl"`
	// +demi:nullable
	Doc *string `json:"doc"`
}

// `POST /providers/setup-token`: a Claude Code entry from a token that
// `claude setup-token` printed.
// +demi:root direction=send output=web
type SetupTokenImport struct {
	// +demi:length chars min=1 max=16384
	Token Trimmed `json:"token"`
	// +demi:length chars min=1 max=80
	Label Trimmed `json:"label"`
}

// `POST /providers/:id/accounts`: another account of an entry, from a setup
// token.
// +demi:root direction=send output=web
type AddToken struct {
	// +demi:length chars min=1 max=16384
	Token Trimmed `json:"token"`
}

// `{ account }`: the account a token import added.
// +demi:root direction=receive output=web
// +demi:tolerant
type AddedAccount struct {
	Account types.AccountInfo `json:"account"`
}

// `GET /providers/:id/accounts`: the entry's accounts and the active one.
// +demi:root direction=receive output=web
// +demi:tolerant
type Accounts struct {
	Accounts []types.AccountInfo `json:"accounts"`
	// +demi:nullable
	Active *CredentialID `json:"active"`
}

// `PUT /providers/:id/accounts/active`.
// +demi:root direction=send output=web
type ActivateAccount struct {
	CredentialID CredentialID `json:"credentialId"`
}

// `{ active }`: the account the entry now infers with.
// +demi:root direction=receive output=web
// +demi:tolerant
type ActiveAccount struct {
	Active CredentialID `json:"active"`
}

// `POST /providers/subscription-login`: the first account of a family's
// entry, by device login.
// +demi:root direction=send output=web
type SubscriptionLogin struct {
	// +demi:length chars min=1
	ProviderType string `json:"providerType"`
	// The new entry's label; the family's subscription name without it.
	// +demi:length chars min=1 max=80
	Label *Trimmed `json:"label,omitempty"`
}

// The 202 answer of a login start: `{ login: { id, status: "pending" } }`.
// +demi:root direction=receive output=web
// +demi:tolerant
type LoginStarted struct {
	Login StartedLogin `json:"login"`
}

// +demi:tolerant
type StartedLogin struct {
	ID     LoginID       `json:"id"`
	Status PendingStatus `json:"status"`
}

// The status of a login that has just started.
// +demi:enum pending
type PendingStatus string

// Values of the preceding enumeration.
const (
	PendingStatusPending PendingStatus = "pending"
)

// `GET /providers/subscription-login/:id`.
// +demi:root direction=receive output=web
// +demi:tolerant
type LoginAnswer struct {
	Login LoginState `json:"login"`
}

// Where a device login is; a finished one is kept for ten minutes.
// +demi:union tag=status
//
//sumtype:decl
type LoginState interface{ loginState() }

// Waiting for the user, who opens the address and enters the code;
// both are null until the vendor names them.
// +demi:variant LoginState pending
// +demi:tolerant
type LoginStatePending struct {
	// +demi:nullable
	VerificationURL *string `json:"verificationUrl"`
	// +demi:nullable
	UserCode *string `json:"userCode"`
	// +demi:nullable
	ExpiresAt *types.Timestamp `json:"expiresAt"`
}

// The login stored its account: `credentialId`, which is the entry's
// active account only when the entry had none.
// +demi:variant LoginState completed
// +demi:tolerant
type LoginStateCompleted struct {
	ProviderID   ProviderID   `json:"providerId"`
	CredentialID CredentialID `json:"credentialId"`
}

// +demi:variant LoginState failed
// +demi:tolerant
type LoginStateFailed struct {
	Message string `json:"message"`
}

// `GET /models`: the catalog of every entry the caller infers with.
// +demi:root direction=receive output=web
// +demi:tolerant
type ModelCatalog struct {
	Providers []CatalogProvider `json:"providers"`
}

// One entry's catalog with the provider's health, read when the backend
// answers.
// +demi:tolerant
type CatalogProvider struct {
	ProviderID  ProviderID `json:"providerId"`
	DisplayName string     `json:"displayName"`
	// The command package that installs the CLI the provider's requests
	// start on the user's Cloud; null when its requests are HTTP.
	// +demi:nullable
	CLIPackage *string        `json:"cliPackage"`
	Models     []CatalogModel `json:"models"`
	// When the source was last downloaded; the Unix epoch for a configured
	// or built-in list, and for a catalog no refresh has filled.
	SourceFetchedAt types.Timestamp `json:"sourceFetchedAt"`
	// Whether this is a copy kept after a failed refresh.
	Stale        bool               `json:"stale"`
	Warnings     []string           `json:"warnings"`
	Auth         types.AuthState    `json:"auth"`
	Runtime      types.RuntimeState `json:"runtime"`
	Availability Availability       `json:"availability"`
}

// A catalog model with the selection the backend built from it, so the
// web app never converts one itself.
// +demi:tolerant
type CatalogModel struct {
	types.ProviderModel
	Selection types.ModelSelection `json:"selection"`
	// The thinking effort a conversation's model settings hold on this
	// model when a change names none: null, no thinking setting, for a
	// model that can turn thinking off (`models.md` § A conversation's
	// model settings).
	// +demi:nullable
	UnnamedEffort *string `json:"unnamedEffort"`
}

// Whether the entry's models can be used now, from its health.
// +demi:union tag=type
//
//sumtype:decl
type Availability interface{ availability() }

// +demi:variant Availability available
// +demi:tolerant
type AvailabilityAvailable struct{}

// +demi:variant Availability unavailable
// +demi:tolerant
type AvailabilityUnavailable struct {
	Reason  UnavailableReason `json:"reason"`
	Message string            `json:"message"`
}

// What makes an entry's models unavailable.
// +demi:enum authentication runtime
type UnavailableReason string

// Values of the preceding enumeration.
const (
	// The credential is missing or refused.
	UnavailableReasonAuthentication UnavailableReason = "authentication"
	// The provider cannot run requests.
	UnavailableReasonRuntime UnavailableReason = "runtime"
)

// `GET /providers/:id/cli`: the command-line tool of an entry whose
// provider runs one on the user's Cloud. Reading it wakes nothing.
// +demi:root direction=receive output=web
// +demi:tolerant
type ProviderCLI struct {
	Newest NewestVersion `json:"newest"`
	// The last install on the caller's Cloud that no conversation asked
	// for, since the backend started; null before one.
	// +demi:nullable
	Install CLIInstall `json:"install"`
	// The caller's Cloud while its runner is connected; empty otherwise.
	Machines []CLIMachine `json:"machines"`
}

// The vendor's newest version, or why it could not be read.
// +demi:union tag=type
//
//sumtype:decl
type NewestVersion interface{ newestVersion() }

// +demi:variant NewestVersion read
// +demi:tolerant
type NewestVersionRead struct {
	Version string `json:"version"`
}

// +demi:variant NewestVersion unreadable
// +demi:tolerant
type NewestVersionUnreadable struct {
	Message string `json:"message"`
}

// Where an install of the tool stands.
// +demi:union tag=state
//
//sumtype:decl
type CLIInstall interface{ cliInstall() }

// +demi:variant CLIInstall installing
// +demi:tolerant
type CLIInstallInstalling struct{}

// `path` is the executable on the Cloud.
// +demi:variant CLIInstall installed
// +demi:tolerant
type CLIInstallInstalled struct {
	Path string `json:"path"`
}

// +demi:variant CLIInstall failed
// +demi:tolerant
type CLIInstallFailed struct {
	Message string `json:"message"`
}

// A machine the tool runs on, with the versions it has, newest first;
// null when it did not answer.
// +demi:tolerant
type CLIMachine struct {
	DeviceID DeviceID `json:"deviceId"`
	Name     string   `json:"name"`
	// +demi:nullable
	Versions *[]string `json:"versions"`
}

// The 202 answer of `POST /providers/:id/cli/install`.
// +demi:root direction=receive output=web
// +demi:tolerant
type CLIInstallAnswer struct {
	Install CLIInstall `json:"install"`
}

// An entry's complete manual model list: 1 to 1000 models with distinct
// ids.
// +demi:root
// +demi:length min=1 max=1000
// +demi:check validateConfiguredModels
type ConfiguredModels []ConfiguredModel

// +demi:length chars min=1 max=64
type ThinkingEffort string

func validateConfiguredModel(model ConfiguredModel) error {
	if model.OutputLimit != nil && *model.OutputLimit > model.ContextWindow {
		return errors.New("the output limit exceeds the context window")
	}
	return nil
}

func validateConfiguredModels(models ConfiguredModels) error {
	seen := make(map[Trimmed]bool, len(models))
	for _, model := range models {
		if seen[model.ID] {
			return fmt.Errorf("the model id %q appears twice", model.ID)
		}
		seen[model.ID] = true
	}
	return nil
}
