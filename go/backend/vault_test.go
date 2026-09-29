package backend_test

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json/v2"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/gates"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// must is value, and fails the test run when err says there is none.
func must[T any](value T, err error) T {
	if err != nil {
		panic(err)
	}
	return value
}

func vaultKey(t *testing.T, fill byte) backend.VaultKey {
	return must(backend.NewVaultKey(bytes.Repeat([]byte{fill}, 32)))
}

// openVault opens a control database with a master account, and its vault
// on an isolated instance.
func openVault(t *testing.T) (*backend.Vault, *storage.Control, webapi.UserID) {
	t.Helper()
	control := must(storage.OpenControl(t.Context(), filepath.Join(t.TempDir(), "control.sqlite"), core.SystemClock{}))
	t.Cleanup(func() { control.Close() })
	hash := must(storage.ParsePasswordHash("$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$0mUbQTTMhhaEBFGMq7WTZxOlVoS9sY3qVqLiV7Q1Izo"))
	master := must(control.CreateMaster(t.Context(), must(webapi.ParseEmailAddress("master@example.test")), hash))
	vault := backend.NewVault(control, vaultKey(t, 3), webapi.InstanceModeIsolated, &backend.SyncRegistry{})
	return vault, control, master.ID
}

// A data directory the Rust backend wrote opens in Go: the same instance
// secret derives the same keys, and each value the Rust vault sealed opens
// for its own row only (storage.md § Passwords and credentials at rest).
// Cost: one small file.
func TestTheRustBackendsSealedCredentialsOpen(t *testing.T) {
	var fixture struct {
		InstanceSecret string `json:"instanceSecret"`
		EmailCodeKey   string `json:"emailCodeKey"`
		Provider       string `json:"provider"`
		Account        string `json:"account"`
		Config         string `json:"config"`
		SealedConfig   string `json:"sealedConfig"`
		Secret         string `json:"secret"`
		SealedSecret   string `json:"sealedSecret"`
	}
	if err := json.Unmarshal(must(os.ReadFile("testdata/rust-vault.json")), &fixture); err != nil {
		t.Fatal(err)
	}
	secret := must(backend.ParseInstanceSecret(fixture.InstanceSecret))
	if got := hex.EncodeToString(secret.EmailCodeKey()); got != fixture.EmailCodeKey {
		t.Fatalf("email code key %s, the Rust derived %s", got, fixture.EmailCodeKey)
	}
	key := secret.VaultKey()
	entry := must(webapi.ParseProviderID(fixture.Provider))
	account := must(webapi.ParseCredentialID(fixture.Account))
	config := must(hex.DecodeString(fixture.SealedConfig))
	document := must(hex.DecodeString(fixture.SealedSecret))
	if opened := must(key.Open(backend.ConfigRow(entry), config)); string(opened) != fixture.Config {
		t.Fatalf("configuration %q", opened)
	}
	if opened := must(key.Open(backend.SecretRow(entry, account), document)); string(opened) != fixture.Secret {
		t.Fatalf("secret %q", opened)
	}
	if _, err := key.Open(backend.SecretRow(entry, account), config); !errors.Is(err, backend.ErrUnsealable) {
		t.Fatalf("a configuration opened as an account's secret: %v", err)
	}
	if _, err := key.Open(backend.ConfigRow(entry), document); !errors.Is(err, backend.ErrUnsealable) {
		t.Fatalf("an account's secret opened as a configuration: %v", err)
	}
}

// A sealed value opens only for its row under its key: altered, moved,
// cut short or sealed under another key, it does not open.
// Cost: in memory.
func TestASealedValueOpensOnlyForItsRowUnderItsKey(t *testing.T) {
	key := vaultKey(t, 7)
	entry := must(webapi.ParseProviderID("entry-1"))
	row := backend.ConfigRow(entry)
	sealed := key.Seal(row, []byte(`{"apiKey":"sk-test-123"}`))
	if bytes.Contains(sealed, []byte("sk-test-123")) {
		t.Fatal("the sealed value holds the key")
	}
	if opened := must(key.Open(row, sealed)); string(opened) != `{"apiKey":"sk-test-123"}` {
		t.Fatalf("opened %q", opened)
	}
	// A fresh nonce every time.
	if bytes.Equal(key.Seal(row, []byte("same")), key.Seal(row, []byte("same"))) {
		t.Fatal("two seals of one value are equal")
	}
	tampered := bytes.Clone(sealed)
	tampered[len(tampered)-1] ^= 1
	refused := map[string]struct {
		key    backend.VaultKey
		row    backend.SealedRow
		sealed []byte
	}{
		"another entry":    {key, backend.ConfigRow(must(webapi.ParseProviderID("entry-2"))), sealed},
		"an account":       {key, backend.SecretRow(entry, must(webapi.ParseCredentialID("cred-1"))), sealed},
		"another key":      {vaultKey(t, 8), row, sealed},
		"a changed byte":   {key, row, tampered},
		"a shorter value":  {key, row, sealed[:8]},
		"no value at all":  {key, row, nil},
		"moved characters": {key, secretRow(t, "a", "bc"), key.Seal(secretRow(t, "ab", "c"), []byte("x"))},
	}
	for name, open := range refused {
		if _, err := open.key.Open(open.row, open.sealed); !errors.Is(err, backend.ErrUnsealable) {
			t.Errorf("%s: %v", name, err)
		}
	}
}

func secretRow(t *testing.T, entry, account string) backend.SealedRow {
	return backend.SecretRow(must(webapi.ParseProviderID(entry)), must(webapi.ParseCredentialID(account)))
}

// The data directory's instance secret is created once, readable by its
// owner only, and read back unchanged; a file that is not 64 hexadecimal
// digits is refused, naming the file. Formatting a secret never shows it.
// Cost: one small file.
func TestTheInstanceSecretFileIsCreatedOnceForItsOwnerOnly(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "instance-secret")
	first := must(backend.LoadInstanceSecret(dir))
	info := must(os.Stat(path))
	if runtime.GOOS != "windows" && info.Mode().Perm() != 0o600 {
		t.Fatalf("permissions %v", info.Mode().Perm())
	}
	again := must(backend.LoadInstanceSecret(dir))
	if !bytes.Equal(first.EmailCodeKey(), again.EmailCodeKey()) {
		t.Fatal("the secret changed when read again")
	}
	digits := strings.TrimSpace(string(must(os.ReadFile(path))))
	for _, verb := range []string{"%v", "%+v", "%#v", "%s", "%x"} {
		if shown := fmt.Sprintf(verb, first); strings.Contains(shown, digits[:8]) {
			t.Fatalf("%s shows the secret: %s", verb, shown)
		}
	}
	if err := os.WriteFile(path, []byte("not hex\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, err := backend.LoadInstanceSecret(dir)
	if want := "the instance secret file " + path + " is not 64 hexadecimal digits"; err == nil || err.Error() != want {
		t.Fatalf("%v, want %s", err, want)
	}
}

// A configured instance secret is 64 hexadecimal digits in either case.
// Cost: in memory.
func TestAConfiguredInstanceSecretIs64HexDigits(t *testing.T) {
	digits := strings.Repeat("0123456789abcdef", 4)
	for _, text := range []string{digits, strings.ToUpper(digits)} {
		must(backend.ParseInstanceSecret(text))
	}
	for _, text := range []string{digits[1:], digits[2:] + "zz", digits + "00", ""} {
		if _, err := backend.ParseInstanceSecret(text); !errors.Is(err, backend.ErrMalformedSecret) {
			t.Errorf("%q: %v", text, err)
		}
	}
}

// An API-key entry's configuration opens when the entry is read, and one
// that does not open or decode is corrupt: the read names the column and the
// fault, never the key, and nothing repairs the value.
// Cost: one SQLite database.
func TestAConfigurationThatDoesNotOpenOrDecodeIsCorrupt(t *testing.T) {
	vault, control, master := openVault(t)
	ctx := t.Context()
	config := backend.APIKeyConfig{APIKey: must(provider.NewSecret("sk-ant-fixture-1"))}
	work := must(vault.CreateAPIKey(ctx, master, "anthropic", "Work", config))
	read := must(vault.Entry(ctx, work.ID))
	if got := read.Credential.(backend.APIKeyConfig).APIKey.Expose(); got != "sk-ant-fixture-1" {
		t.Fatalf("key %q", got)
	}
	other := must(vault.CreateAPIKey(ctx, master, "openai", "Other", config))
	stored := must(control.Provider(ctx, other.ID))
	unknown := vaultKey(t, 3).Seal(backend.ConfigRow(work.ID), []byte(`{"apiKey":"sk-ant-fixture-1","extra":1}`))
	notText := vaultKey(t, 3).Seal(backend.ConfigRow(work.ID), []byte{0xff, 0xfe})
	cases := map[string]struct {
		sealed []byte
		reason string
	}{
		"another entry's configuration": {stored.Config, "the sealed value does not open"},
		"a member it does not name":     {unknown, "malformed at"},
		"bytes that are not text":       {notText, "the configuration is not UTF-8"},
	}
	for name, corrupt := range cases {
		must(control.UpdateProvider(ctx, work.ID, nil, corrupt.sealed))
		_, err := vault.Entry(ctx, work.ID)
		var refused *storage.CorruptError
		if !errors.As(err, &refused) || refused.Table != "providers" || refused.Column != "config" || !strings.Contains(refused.Reason, corrupt.reason) {
			t.Errorf("%s: %v", name, err)
		}
		if err != nil && strings.Contains(err.Error(), "sk-ant") {
			t.Errorf("%s: the refusal shows the key: %v", name, err)
		}
		if _, err := vault.Entries(ctx, master); err == nil {
			t.Errorf("%s: the owner's entries were read", name)
		}
	}
}

// tokens is a family's secret document in the pool test.
type tokens struct {
	Refresh string `json:"refresh"`
}

func decodeTokens(data []byte) (tokens, error) {
	var value tokens
	err := json.Unmarshal(data, &value, json.RejectUnknownMembers(true))
	return value, err
}

func encodeTokens(value tokens) ([]byte, error) { return json.Marshal(value) }

// turnWatch is an account document that says when its holder asks for its
// refresh turn.
type turnWatch struct {
	provider.AccountDocument
	asked chan struct{}
}

func (w turnWatch) RefreshTurn(ctx context.Context) (*gates.KeyedPermit[string], error) {
	close(w.asked)
	return w.AccountDocument.RefreshTurn(ctx)
}

func account(id string) provider.AccountMeta {
	key := id
	return provider.AccountMeta{ID: id, Label: id + "@example.test", Source: "login:device", IdentityKey: &key}
}

// An entry's account is a record whose secret is sealed to its row and
// replaced only over the version it was read at; concurrent renewals of one
// account ask the vendor once; and an entry's pool reaches no other entry's
// accounts (providers.md § The credential pool contract).
// Cost: one SQLite database.
func TestAnAccountIsASealedRecordRefreshedOnlyOverTheVersionItWasReadAt(t *testing.T) {
	vault, control, master := openVault(t)
	ctx := t.Context()
	staged := provider.NewMemoryCredentialPool()
	entry := must(vault.CreateSubscription(ctx, master, "codex", "Codex", staged))
	pool := vault.Pool(entry.ID)
	if active := must(pool.Active(ctx)); active != nil {
		t.Fatalf("active %s", *active)
	}
	if err := pool.Write(ctx, account("a"), `{"refresh":"one"}`); err != nil {
		t.Fatal(err)
	}
	// The first account of an entry without an active one becomes it, in
	// the same write.
	if active := must(pool.Active(ctx)); active == nil || *active != "a" {
		t.Fatalf("active %v", active)
	}
	rows := must(control.Credentials(ctx, entry.ID))
	if bytes.Contains(rows[0].Secret, []byte("one")) {
		t.Fatal("the record holds the secret")
	}

	// Two refreshers read one revision; the second write finds a newer one.
	document := pool.Document("a")
	first := must(document.Read(ctx))
	second := must(document.Read(ctx))
	if first.Text != `{"refresh":"one"}` {
		t.Fatalf("read %q", first.Text)
	}
	if !must(document.Replace(ctx, `{"refresh":"two"}`, first.Version)) {
		t.Fatal("the replace over the version read was refused")
	}
	if must(document.Replace(ctx, `{"refresh":"lost"}`, second.Version)) {
		t.Fatal("a replace over an older version was stored")
	}
	if text := must(document.Read(ctx)).Text; text != `{"refresh":"two"}` {
		t.Fatalf("read %q", text)
	}

	// Concurrent renewals of one account ask the vendor once, and the
	// second uses what the first stored: the first refreshes only once the
	// second waits for its turn.
	var asked atomic.Int32
	waiting := turnWatch{pool.Document("a"), make(chan struct{})}
	stillDue := func(value tokens) bool { return value.Refresh == "two" }
	var renewed sync.WaitGroup
	var later tokens
	var laterErr error
	refresh := func(ctx context.Context, value tokens) (tokens, error) {
		asked.Add(1)
		renewed.Go(func() {
			later, laterErr = provider.Renew(ctx, waiting, decodeTokens, encodeTokens, stillDue, func(context.Context, tokens) (tokens, error) {
				asked.Add(1)
				return tokens{Refresh: "unwanted"}, nil
			})
		})
		<-waiting.asked
		return tokens{Refresh: value.Refresh + "+"}, nil
	}
	earlier := must(provider.Renew(ctx, pool.Document("a"), decodeTokens, encodeTokens, stillDue, refresh))
	renewed.Wait()
	if laterErr != nil || earlier.Refresh != "two+" || later.Refresh != "two+" || asked.Load() != 1 {
		t.Fatalf("renewed %q and %q (%v), asking %d times", earlier.Refresh, later.Refresh, laterErr, asked.Load())
	}

	// Another entry's pool reaches none of these accounts.
	other := must(vault.CreateSubscription(ctx, master, "grok-build", "Grok", staged))
	foreign := vault.Pool(other.ID)
	if revision := must(foreign.Document("a").Read(ctx)); revision != nil {
		t.Fatalf("another entry's pool read %q", revision.Text)
	}
	if listed := must(foreign.List(ctx)); len(listed) != 0 {
		t.Fatalf("another entry's pool lists %v", listed)
	}
	// A secret moved into another entry's row does not open there.
	moved := must(control.Credential(ctx, entry.ID, must(webapi.ParseCredentialID("a"))))
	if _, err := control.WriteCredential(ctx, other.ID, moved.CredentialWrite); err != nil {
		t.Fatal(err)
	}
	if _, err := foreign.Document("a").Read(ctx); err == nil || err.Error() != "the secret of account a does not open" {
		t.Fatalf("a moved secret read: %v", err)
	}

	if err := pool.Write(ctx, account("b"), "{}"); err != nil {
		t.Fatal(err)
	}
	if err := pool.SetActive(ctx, "missing"); !errors.Is(err, provider.PoolNotFound{ID: "missing"}) {
		t.Fatalf("selecting a missing account: %v", err)
	}
	if err := pool.SetActive(ctx, "b"); err != nil {
		t.Fatal(err)
	}
	if err := pool.Remove(ctx, "b"); err != nil {
		t.Fatal(err)
	}
	if active := must(pool.Active(ctx)); active != nil {
		t.Fatalf("the removed active account is still active: %s", *active)
	}
	if err := vault.Delete(ctx, *entry); err != nil {
		t.Fatal(err)
	}
	if left := must(control.Credentials(ctx, entry.ID)); len(left) != 0 {
		t.Fatalf("a deleted entry's accounts are left: %v", left)
	}
}
