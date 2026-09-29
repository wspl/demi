package backend

import (
	"errors"
	"maps"
	"net/http"
	"net/url"
	"slices"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/anthropicapi"
	"github.com/wspl/demi/go/provider/claudecode"
	"github.com/wspl/demi/go/provider/codex"
	"github.com/wspl/demi/go/provider/google"
	"github.com/wspl/demi/go/provider/grokbuild"
	"github.com/wspl/demi/go/provider/openaiapi"
	"github.com/wspl/demi/go/webapi"
)

// ErrWrongCredential refuses to build a provider for an entry whose
// credential is not the kind its family takes.
var ErrWrongCredential = errors.New("the entry's credential is not one its family takes")

// ProviderFamily is a provider family: anthropic, codex, or a test's
// scripted one (providers.md § Families, vendors and endpoints). A family
// says how its entries authenticate and builds the provider of one entry
// and account from the entry's decoded configuration.
type ProviderFamily interface {
	// Credential is how the family's entries authenticate.
	Credential() webapi.CredentialKind
	// Wires are the wires an entry of the family may choose; none for a
	// family that speaks one.
	Wires() []core.WireAPI
	// Provider is the provider of one entry and account.
	Provider(args FamilyArgs) (provider.Provider, error)
}

// FamilyArgs are what a family builds a provider from.
type FamilyArgs struct {
	// EntryID is the entry's id, which a provider that logs its runs names
	// them by.
	EntryID string
	Label   string
	// Credential is an APIKeyArgs or a SubscriptionArgs.
	Credential FamilyCredential
	// HTTP is the client of the backend's shared services, for the
	// provider's own requests such as its directory and quota probes. A
	// runtime gets the client of the shard it runs on instead.
	HTTP  *http.Client
	Clock core.Clock
	// ModelsDev is the backend's one models.dev copy, for a family whose
	// directory it is.
	ModelsDev *provider.ModelsDevClient
}

// FamilyCredential is the credential a provider stands for: an APIKeyArgs
// or a SubscriptionArgs.
type FamilyCredential interface{ familyCredential() }

// APIKeyArgs are an API-key entry's settings, read only from the entry.
type APIKeyArgs struct {
	APIKey provider.Secret
	// BaseURL is the vendor's API base; nil for the family's default.
	BaseURL *url.URL
	WireAPI *core.WireAPI
	// Vendor holds the request requirements of the vendor the entry was
	// added from, which the OpenAI wires apply.
	Vendor openaiapi.VendorPolicy
}

// SubscriptionArgs are a subscription entry's accounts: the pool bound to
// the entry, and the account the provider stands for, when the entry has
// one.
type SubscriptionArgs struct {
	Pool    provider.CredentialPool
	Account *AccountBinding
}

// AccountBinding is the account a subscription provider stands for, with
// the store of its quota snapshot.
type AccountBinding struct {
	CredentialID string
	Quota        provider.QuotaSnapshotStore
}

func (APIKeyArgs) familyCredential()       {}
func (SubscriptionArgs) familyCredential() {}

// FamilyRegistry holds the families of a backend by name. A registry is
// never changed: With answers a new one.
type FamilyRegistry struct{ families map[string]ProviderFamily }

// BuiltinFamilies are the families built into the backend.
func BuiltinFamilies() FamilyRegistry {
	return FamilyRegistry{}.
		With("anthropic", anthropicFamily{}).
		With(setupTokenFamily, claudeCodeFamily{}).
		With("codex", CodexFamily{}).
		With("google", googleFamily{}).
		With("grok-build", grokBuildFamily{}).
		With("openai", openAIFamily{})
}

// With is the registry with family under name, in place of any family of
// that name.
func (r FamilyRegistry) With(name string, family ProviderFamily) FamilyRegistry {
	families := maps.Clone(r.families)
	if families == nil {
		families = map[string]ProviderFamily{}
	}
	families[name] = family
	return FamilyRegistry{families}
}

// Get is the family name, if the registry has it.
func (r FamilyRegistry) Get(name string) (ProviderFamily, bool) {
	family, ok := r.families[name]
	return family, ok
}

// Subscriptions are the names of the subscription families, in order.
func (r FamilyRegistry) Subscriptions() []string {
	var names []string
	for _, name := range slices.Sorted(maps.Keys(r.families)) {
		if r.families[name].Credential() == webapi.CredentialKindSubscription {
			names = append(names, name)
		}
	}
	return names
}

// built is a family package's new provider as the interface, and no
// provider at all when the package refused to build one.
func built[P provider.Provider](made P, err error) (provider.Provider, error) {
	if err != nil {
		return nil, err
	}
	return made, nil
}

// bound is the account a subscription provider stands for and the store of
// its quota. A provider without an account, such as one built to log in,
// can neither infer nor probe, so its quota store is one held in memory
// that nothing writes.
func bound(account *AccountBinding) (*string, provider.QuotaSnapshotStore) {
	if account == nil {
		return nil, &provider.MemorySnapshots{}
	}
	return &account.CredentialID, account.Quota
}

// anthropicFamily is the Anthropic Messages API with an API key.
type anthropicFamily struct{}

func (anthropicFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindAPIKey }
func (anthropicFamily) Wires() []core.WireAPI             { return nil }
func (anthropicFamily) Provider(args FamilyArgs) (provider.Provider, error) {
	settings, ok := args.Credential.(APIKeyArgs)
	if !ok {
		return nil, ErrWrongCredential
	}
	return built(anthropicapi.New(anthropicapi.Config{APIKey: settings.APIKey, BaseURL: settings.BaseURL}, args.Clock))
}

// openAIFamily is the Responses API, or Chat Completions for an
// OpenAI-compatible endpoint, with an API key.
type openAIFamily struct{}

func (openAIFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindAPIKey }
func (openAIFamily) Wires() []core.WireAPI {
	return []core.WireAPI{core.WireAPIResponses, core.WireAPIChatCompletions}
}
func (openAIFamily) Provider(args FamilyArgs) (provider.Provider, error) {
	settings, ok := args.Credential.(APIKeyArgs)
	if !ok {
		return nil, ErrWrongCredential
	}
	// An entry that names no wire speaks Responses.
	wire := core.WireAPIResponses
	if settings.WireAPI != nil {
		wire = *settings.WireAPI
	}
	config := openaiapi.Config{APIKey: settings.APIKey, BaseURL: settings.BaseURL, Wire: wire, Policy: settings.Vendor}
	return built(openaiapi.New(config, args.Clock))
}

// googleFamily is Gemini's generateContent API with an API key.
type googleFamily struct{}

func (googleFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindAPIKey }
func (googleFamily) Wires() []core.WireAPI             { return nil }
func (googleFamily) Provider(args FamilyArgs) (provider.Provider, error) {
	settings, ok := args.Credential.(APIKeyArgs)
	if !ok {
		return nil, ErrWrongCredential
	}
	return built(google.New(google.Config{APIKey: settings.APIKey, BaseURL: settings.BaseURL}, args.Clock))
}

// CodexFamily is the codex family: ChatGPT accounts on the Codex backend,
// signed in by Codex's device login. Endpoints, when set, put a test's
// scripted Codex in place of the vendor's own.
type CodexFamily struct{ Endpoints *CodexEndpoints }

// CodexEndpoints are the ChatGPT backend and the sign-in service of the
// codex family.
type CodexEndpoints struct{ Backend, Auth url.URL }

func (CodexFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindSubscription }
func (CodexFamily) Wires() []core.WireAPI             { return nil }
func (f CodexFamily) Provider(args FamilyArgs) (provider.Provider, error) {
	subscription, ok := args.Credential.(SubscriptionArgs)
	if !ok {
		return nil, ErrWrongCredential
	}
	account, quota := bound(subscription.Account)
	config := codex.NewConfig(account)
	if f.Endpoints != nil {
		config.BackendURL = f.Endpoints.Backend
		config.AuthURL = f.Endpoints.Auth
	}
	return built(codex.New(config, subscription.Pool, quota, args.HTTP, args.Clock))
}

// grokBuildFamily is Grok accounts on the chat proxy of Grok's own CLI,
// signed in by device authorization at auth.x.ai.
type grokBuildFamily struct{}

func (grokBuildFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindSubscription }
func (grokBuildFamily) Wires() []core.WireAPI             { return nil }
func (grokBuildFamily) Provider(args FamilyArgs) (provider.Provider, error) {
	subscription, ok := args.Credential.(SubscriptionArgs)
	if !ok {
		return nil, ErrWrongCredential
	}
	account, quota := bound(subscription.Account)
	return built(grokbuild.New(grokbuild.NewConfig(account), subscription.Pool, quota, args.HTTP, args.Clock))
}

// claudeCodeFamily is Claude accounts by setup token, whose requests run
// the Claude Code CLI on the user's Cloud.
type claudeCodeFamily struct{}

func (claudeCodeFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindSubscription }
func (claudeCodeFamily) Wires() []core.WireAPI             { return nil }
func (claudeCodeFamily) Provider(args FamilyArgs) (provider.Provider, error) {
	subscription, ok := args.Credential.(SubscriptionArgs)
	if !ok {
		return nil, ErrWrongCredential
	}
	account, quota := bound(subscription.Account)
	config := claudecode.NewConfig(args.EntryID, args.Label, account)
	return claudecode.New(config, subscription.Pool, quota, args.ModelsDev, args.HTTP, args.Clock), nil
}
