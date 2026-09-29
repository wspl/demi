package provider_test

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/codex"
	"github.com/wspl/demi/go/provider/grokbuild"
	"github.com/wspl/demi/go/provider/providertest"
)

// Cost: loopback token endpoints only; refresh coordination is exercised once
// by the shared credential tests, and family expiry/refusal rules here.
func TestSubscriptionExpiryAndAuthenticationFailures(t *testing.T) {
	for _, family := range []string{"codex", "grok"} {
		t.Run(family, func(t *testing.T) {
			for _, scenario := range []string{"expiring", "email", "old-refresh", "refresh-refused", "second-401", "corrupt", "absent", "no-refresh"} {
				t.Run(scenario, func(t *testing.T) {
					if (family == "codex" && scenario == "no-refresh") || (family == "grok" && scenario == "old-refresh") {
						return
					}
					scripts := []providertest.MockResponse{{Chunks: []string{`{"access_token":"fresh","expires_in":3600}`}}, {}}
					if scenario == "refresh-refused" {
						scripts = []providertest.MockResponse{{Status: 503, Chunks: []string{"do not expose refresh details"}}}
					}
					if scenario == "second-401" {
						scripts = []providertest.MockResponse{{Status: 401}, {Chunks: []string{`{"access_token":"fresh","expires_in":3600}`}}, {Status: 401}}

					}
					if scenario == "no-refresh" {
						scripts = []providertest.MockResponse{{}}
					}
					vendor := providertest.NewMockVendor(t, scripts...)
					pool := provider.NewMemoryCredentialPool()
					access, err := provider.NewSecret(providertest.JWT(jsontext.Value(`{"exp":200,"https://api.openai.com/auth":{"chatgpt_account_is_fedramp":true}}`)))
					if err != nil {
						t.Fatal(err)
					}
					if scenario == "second-401" || scenario == "old-refresh" {
						access, err = provider.NewSecret("access")
						if err != nil {
							t.Fatal(err)
						}
					}
					refresh, err := provider.NewSecret("refresh")
					if err != nil {
						t.Fatal(err)
					}
					base, err := url.Parse(vendor.Server.URL)
					if err != nil {
						t.Fatal(err)
					}
					var raw []byte
					var p provider.Provider
					clock := providertest.FixedClock{Time: core.UnixEpoch}
					if scenario == "old-refresh" {
						clock.Time = core.TruncateTimestamp(core.UnixEpoch.Time().Add(8 * 24 * time.Hour))
					}
					if family == "codex" {
						idToken := refresh
						if scenario == "email" {
							idToken, err = provider.NewSecret(providertest.JWT(jsontext.Value(`{"email":"credential@example.test"}`)))
							if err != nil {
								t.Fatal(err)
							}
						}
						raw, err = (codex.SecretDocument{AccessToken: access, RefreshToken: refresh, IDToken: idToken, AccountID: "account", LastRefresh: core.UnixEpoch}).JSON()
						if err != nil {
							t.Fatal(err)
						}
						config := codex.NewConfig(new("one"))
						config.BackendURL = *base
						config.AuthURL = *base
						config.Transport = codex.TransportSSE
						p, err = codex.New(config, pool, &provider.MemorySnapshots{}, vendor.Server.Client(), clock)
					} else {
						document := grokbuild.SecretDocument{AccessToken: access, RefreshToken: &refresh, Issuer: base.String(), ClientID: "client"}
						if scenario == "email" {
							document.Email = new("credential@example.test")
						}
						if scenario == "no-refresh" {
							document.RefreshToken = nil
						}
						raw, err = document.JSON()
						if err != nil {
							t.Fatal(err)
						}
						config := grokbuild.NewConfig(new("one"))
						config.ProxyURL = *base
						config.IssuerURL = *base
						p, err = grokbuild.New(config, pool, &provider.MemorySnapshots{}, vendor.Server.Client(), clock)
					}
					if err != nil {
						t.Fatal(err)
					}
					if scenario == "corrupt" {
						var fields map[string]jsontext.Value
						if err := json.Unmarshal(raw, &fields); err != nil {
							t.Fatal(err)
						}
						fields["accessToken"] = jsontext.Value(`"credential-must-not-leak"`)
						fields["refreshToken"] = jsontext.Value(`7`)
						raw = provider.JSONBody(fields)
					}
					if scenario != "absent" {
						if err := pool.Write(t.Context(), provider.AccountMeta{ID: "one", Label: "account"}, string(raw)); err != nil {
							t.Fatal(err)
						}
					}
					// Status reads the stored document and must not refresh an expiring token.
					state := p.AuthStatus(t.Context())
					switch scenario {
					case "absent":
						value, ok := state.(core.AuthStateUnauthenticated)
						name := "Codex"
						if family == "grok" {
							name = "Grok"
						}
						if !ok || value.Message == nil || *value.Message != "No "+name+" account is signed in" {
							t.Fatalf("missing account status: %#v", state)
						}
					case "corrupt":
						value, ok := state.(core.AuthStateError)
						if !ok || !strings.Contains(value.Message, "refreshToken") || strings.Contains(value.Message, "credential-must-not-leak") {
							t.Fatalf("corrupt account status: %#v", state)
						}
					default:
						want := "account"
						if family == "grok" {
							want = base.String() + "::client"
						}
						if scenario == "email" {
							want = "credential@example.test"
						}
						value, ok := state.(core.AuthStateAuthenticated)
						if !ok || value.AccountLabel == nil || *value.AccountLabel != want {
							t.Fatalf("account status: %#v", state)
						}
						if err := pool.Write(t.Context(), provider.AccountMeta{ID: "other", Label: "other"}, "{}"); err != nil {
							t.Fatal(err)
						}
						if err := pool.SetActive(t.Context(), "other"); err != nil {
							t.Fatal(err)
						}
						changed, ok := p.AuthStatus(t.Context()).(core.AuthStateAuthenticated)
						if !ok || changed.AccountLabel == nil || *changed.AccountLabel != want {
							t.Fatalf("active account changed provider identity: %#v", changed)
						}
					}
					if len(vendor.Requests()) != 0 {
						t.Fatal("status refreshed credentials")
					}
					runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: vendor.Server.Client()})
					if err != nil {
						t.Fatal(err)
					}
					events := providertest.Run(t, runtime, providertest.InferenceRequest())
					if len(events) != 1 {
						t.Fatalf("events %#v", events)
					}
					switch scenario {
					case "expiring", "email", "old-refresh":
						if _, ok := events[0].(provider.Response); !ok || len(vendor.Requests()) != 2 {
							t.Fatalf("refresh %#v %#v", events, vendor.Requests())
						}
					case "no-refresh":
						if _, ok := events[0].(provider.Response); !ok || len(vendor.Requests()) != 1 {
							t.Fatalf("nonrefreshable %#v", events)
						}
					default:
						failure, ok := events[0].(provider.FailureEvent)
						if !ok {
							t.Fatalf("failure %#v", events)
						}
						want := map[string]provider.ErrorCode{"corrupt": provider.AuthInvalid, "absent": provider.AuthMissing, "refresh-refused": provider.AuthRefreshFailed, "second-401": provider.AuthExpired}[scenario]
						if failure.Failure.Code != want {
							t.Fatalf("failure %#v", failure)
						}
						if (scenario == "corrupt" || scenario == "absent") && len(vendor.Requests()) != 0 {
							t.Fatal("invalid account sent a request")
						}
					}
				})
			}
		})
	}
}
