package backend_test

import (
	"context"
	"crypto/sha256"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/cmdpkg/claudecode/claudecodeop"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/webapi"
)

const accountClaudeConversation = "3c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01"
const accountScriptedCLI = `#!/bin/sh
log="$HOME/claude-processes.log"
echo "started $CLAUDE_CODE_OAUTH_TOKEN" >> "$log"
trap 'echo "ended $CLAUDE_CODE_OAUTH_TOKEN" >> "$log"; exit 0' TERM
while IFS= read -r line; do
  case "$line" in
    '{"type":"control_request"'*'"subtype":"initialize"'*)
      id=$(printf '%s\n' "$line" | sed 's/^{"type":"control_request","request_id":"\([^"]*\)".*/\1/')
      printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s"}}\n' "$id"
      ;;
    '{"type":"user"'*)
      printf '{"type":"assistant","message":{"content":[{"type":"text","text":"cli=%s token=%s cwd=%s config=%s"}]}}\n' \
        "$0" "$CLAUDE_CODE_OAUTH_TOKEN" "$(pwd)" "$CLAUDE_CONFIG_DIR"
      printf '{"type":"result","usage":{"input_tokens":3,"output_tokens":2}}\n'
      ;;
  esac
done
`

// accountClaudeModels supplies the same Claude models to scripted and real CLI scenarios.
func accountClaudeModels() providertest.MockResponse {
	response := accountVendorResponse(`{"anthropic":{"id":"anthropic","name":"Anthropic","npm":"@ai-sdk/anthropic","models":{"claude-opus-4-8":{"name":"Claude Opus 4.8","reasoning":true,"tool_call":true,"attachment":true,"reasoning_options":[{"type":"effort","values":["low","medium","high"]}],"limit":{"context":1000000,"output":64000}},"claude-sonnet-4-6":{"name":"Claude Sonnet 4.6","reasoning":true,"tool_call":true,"attachment":true,"reasoning_options":[{"type":"effort","values":["low","medium","high"]}],"limit":{"context":1000000,"output":64000}}}}}`)
	response.Headers = http.Header{"Etag": []string{`"fixture"`}}
	return response
}

// accountClaudeHarness publishes the real command package and owns its manager through backend shutdown.
func accountClaudeHarness(ctx context.Context, t *testing.T) (*backendtest.Harness, *backendtest.ScriptedManager) {
	t.Helper()
	built, err := backendtest.BuildPackage(ctx, t, "demi-claude-code")
	if err != nil {
		t.Fatal(err)
	}
	catalog, err := built.Catalog(ctx)
	if err != nil {
		t.Fatal(err)
	}
	manager, err := backendtest.StartScriptedManager(context.WithoutCancel(ctx), t)
	if err != nil {
		t.Fatal(err)
	}
	h, err := backendtest.NewHarness(ctx, t, manager.Socket())
	if err != nil {
		t.Fatal(err)
	}
	h.Config.Native = catalog
	return h, manager
}

// accountCLISettled observes installation completion through the public settings route.
func accountCLISettled(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session, id string) webapi.ProviderCLI {
	t.Helper()
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	for {
		a := conversationRequest(ctx, t, b, s, "GET", "/api/providers/"+id+"/cli", "", 200)
		cli := conversationDecode(t, a, webapi.DecodeProviderCLI)
		if _, installing := cli.Install.(*webapi.CLIInstallInstalling); !installing {
			return cli
		}
	}
}

type accountLocalArtifact string

func (p accountLocalArtifact) Resolve(context.Context, commandwire.PackageArtifact) (cmdpkgs.ArtifactSource, error) {
	return cmdpkgs.ArtifactSource{Path: string(p)}, nil
}

// accountInstallCLI puts the scripted executable through the runner's verified artifact cache.
func accountInstallCLI(ctx context.Context, t *testing.T, root, version string) string {
	t.Helper()
	source := filepath.Join(t.TempDir(), "claude")
	if err := os.WriteFile(source, []byte(accountScriptedCLI), 0600); err != nil {
		t.Fatal(err)
	}
	installs := &cmdpkgs.Installs{}
	defer installs.Close()
	cache, err := cmdpkgs.NewArtifactCache(ctx, root, "", installs)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := cache.Close(context.WithoutCancel(ctx)); err != nil {
			t.Error(err)
		}
	}()
	digest := fmt.Sprintf("%x", sha256.Sum256([]byte(accountScriptedCLI)))
	path, err := cache.Install(ctx, cmdpkgs.Wanted{Package: claudecodeop.Package, Name: "Claude Code", Version: version, Artifact: commandwire.PackageArtifact{SHA256: digest, Size: uint64(len(accountScriptedCLI))}, Form: &commandwire.ArtifactFile{}}, accountLocalArtifact(source))
	if err != nil {
		t.Fatal(err)
	}
	return path
}

// Several seconds: a real Cloud runner installs the command package and runs its scripted CLI.
func TestAConversationOnAPairedDeviceInfersThroughTheCloudsCLIWithTheActiveAccountsToken(t *testing.T) {
	t.Parallel()
	ctx := t.Context()
	distribution := providertest.StartVendor(t)
	distribution.RespondAt("/releases/latest", accountVendorResponse("2.1.3\n"))
	build := `{"binary":"claude","checksum":"` + strings.Repeat("ab", 32) + `","size":1024}`
	distribution.RespondAt("/releases/2.1.3/manifest.json", accountVendorResponse(`{"version":"2.1.3","platforms":{"linux-x64":`+build+`,"linux-arm64":`+build+`,"darwin-arm64":`+build+`}}`))
	catalog := providertest.StartVendor(t)
	catalog.RespondAt("/api.json", accountClaudeModels())
	h, manager := accountClaudeHarness(ctx, t)
	var err error
	h.Config.ClaudeReleases, err = url.Parse(distribution.URL("/releases"))
	if err != nil {
		t.Fatal(err)
	}
	h.Config.ModelsDevURL, err = url.Parse(catalog.URL("/api.json"))
	if err != nil {
		t.Fatal(err)
	}
	b, master, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	const firstToken = "sk-ant-oat01-first-account"
	const secondToken = "sk-ant-oat01-second-account"
	imported := conversationRequest(ctx, t, b, &master, "POST", "/api/providers/setup-token", `{"token":"`+firstToken+`","label":"Claude"}`, 201)
	id := string(conversationDecode(t, imported, webapi.DecodeProviderAnswer).Provider.ID)
	failed := accountCLISettled(ctx, t, b, &master, id)
	failure, ok := failed.Install.(*webapi.CLIInstallFailed)
	if !ok {
		t.Fatalf("install: %#v", failed.Install)
	}
	reason := "Claude Code 2.1.3 could not be installed: invalid release record"
	if !strings.HasPrefix(failure.Message, reason) {
		t.Fatal(failure.Message)
	}
	conversationEqual[webapi.NewestVersion](t, failed.Newest, &webapi.NewestVersionRead{Version: "2.1.3"})
	devices := manager.Devices()
	conversationEqual(t, len(devices), 1)
	cloud := devices[0]
	conversationEqual(t, len(failed.Machines), 1)
	conversationEqual(t, failed.Machines[0].DeviceID, cloud)
	if failed.Machines[0].Versions == nil {
		t.Fatal("no version report")
	}
	conversationEqual(t, len(*failed.Machines[0].Versions), 0)
	conversationEqual(t, len(conversationDecode(t, conversationRequest(ctx, t, b, &master, "GET", "/api/providers/"+id+"/accounts", "", 200), webapi.DecodeAccounts).Accounts), 1)
	conversationCreate(ctx, t, b, &master, accountClaudeConversation)
	laptop, err := b.Pair(ctx, t, &master, "laptop")
	if err != nil {
		t.Fatal(err)
	}
	root := filepath.Join(laptop.Runner.Home(), "work")
	if err := os.MkdirAll(root, 0755); err != nil {
		t.Fatal(err)
	}
	db, err := h.ControlDatabase(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE conversations SET target_kind = 'device', target_device_id = ?, target_path = ? WHERE id = ?", laptop.ID(), root, accountClaudeConversation); err != nil {
		t.Fatal(err)
	}
	conversationChoose(ctx, t, b, &master, accountClaudeConversation, id, "claude-opus-4-8")
	socket, err := b.Conversation(ctx, t, &master, accountClaudeConversation)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := socket.Open(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := socket.Chat(ctx, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "hello"); err != nil {
		t.Fatal(err)
	}
	blocks := conversationTranscript(ctx, t, b, &master, accountClaudeConversation).Blocks
	last, ok := blocks[len(blocks)-1].(*core.ErrorBlock)
	if !ok {
		t.Fatal("request did not fail")
	}
	if !strings.HasPrefix(last.Message, reason) {
		t.Fatal(last.Message)
	}
	home := manager.Home(cloud)
	older := accountInstallCLI(ctx, t, manager.Artifacts(cloud), "2.1.2")
	conversationRequest(ctx, t, b, &master, "POST", "/api/providers/"+id+"/cli/install", "{}", 202)
	installed := accountCLISettled(ctx, t, b, &master, id)
	conversationEqual[webapi.CLIInstall](t, installed.Install, &webapi.CLIInstallInstalled{Path: older})
	conversationEqual(t, *installed.Machines[0].Versions, []string{"2.1.2"})
	conversationEqual(t, len(distribution.Requests()), 2)
	answer := func(token string) string {
		return fmt.Sprintf("cli=%s token=%s cwd=%s/.demi/claude/run config=%s/.demi/claude/config", older, token, home, home)
	}
	if _, err := socket.Chat(ctx, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a02", "hello again"); err != nil {
		t.Fatal(err)
	}
	conversationEqual(t, conversationLastText(t, conversationTranscript(ctx, t, b, &master, accountClaudeConversation).Blocks), answer(firstToken))
	if _, err := os.Stat(filepath.Join(laptop.Runner.Home(), ".demi/claude")); !os.IsNotExist(err) {
		t.Fatalf("laptop CLI directory: %v", err)
	}
	added := conversationRequest(ctx, t, b, &master, "POST", "/api/providers/"+id+"/accounts", `{"token":"`+secondToken+`"}`, 201)
	second := conversationDecode(t, added, webapi.DecodeAddedAccount).Account.ID
	conversationRequest(ctx, t, b, &master, "PUT", "/api/providers/"+id+"/accounts/active", `{"credentialId":"`+second+`"}`, 200)
	if _, err := socket.Chat(ctx, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a03", "and again"); err != nil {
		t.Fatal(err)
	}
	conversationEqual(t, conversationLastText(t, conversationTranscript(ctx, t, b, &master, accountClaudeConversation).Blocks), answer(secondToken))
	processes, err := os.ReadFile(filepath.Join(home, "claude-processes.log"))
	if err != nil {
		t.Fatal(err)
	}
	conversationEqual(t, strings.Split(strings.TrimSuffix(string(processes), "\n"), "\n"), []string{"started " + firstToken, "ended " + firstToken, "started " + secondToken})
}
