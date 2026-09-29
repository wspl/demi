package backendtest_test

import (
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

const (
	claudeConversation = "3c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	// claudeNewest is the version the distribution names as its newest, and
	// claudeOlder the version the Cloud has.
	claudeNewest      = "2.1.3"
	claudeOlder       = "2.1.2"
	claudeFirstToken  = "sk-ant-oat01-first-account"
	claudeSecondToken = "sk-ant-oat01-second-account"
)

// scriptedCLI is the Claude Code CLI as a script: it answers initialize, answers
// each user message with the executable, the token, the directory it runs in and
// its configuration home, and records its start and its end.
const scriptedCLI = `#!/bin/sh
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

// claudeManifest is the distribution's manifest of version.
func claudeManifest(version string) *scripted.Response {
	build := backendtest.Map{"binary": "claude", "checksum": strings.Repeat("ab", 32), "size": 1024}
	return scripted.Status(200).Header("content-type", "application/json").Chunk(backendtest.Marshal(backendtest.Map{
		"version":   version,
		"platforms": backendtest.Map{"linux-x64": build, "linux-arm64": build, "darwin-arm64": build},
	}))
}

// installByHand puts version of the scripted CLI in the Cloud's home, with the
// receipt an install leaves, and answers the executable's path.
func installByHand(t *testing.T, home, version string) string {
	t.Helper()
	directory := filepath.Join(home, ".demi", "claude", version)
	if err := os.MkdirAll(directory, 0o755); err != nil {
		t.Fatal(err)
	}
	executable := filepath.Join(directory, "claude")
	if err := os.WriteFile(executable, []byte(scriptedCLI), 0o755); err != nil {
		t.Fatal(err)
	}
	receipt := backendtest.Marshal(backendtest.Map{
		"version": version, "platform": "linux-x64", "sha256": strings.Repeat("ab", 32), "size": len(scriptedCLI),
	})
	if err := os.WriteFile(filepath.Join(directory, "receipt.json"), receipt, 0o644); err != nil {
		t.Fatal(err)
	}
	return executable
}

// settledCLI is the entry's CLI once no install is under way.
func settledCLI(t *testing.T, b *backendtest.Backend, session *backendtest.Session, provider string) *backendtest.Answer {
	t.Helper()
	var read *backendtest.Answer
	backendtest.Eventually(t, "the install ends", func() bool {
		read = b.Get("/api/providers/"+provider+"/cli", session).Expect(http.StatusOK)
		return read.Str("install.state") != "installing"
	})
	return read
}

// claudeModels is a models.dev document whose anthropic vendor lists the Claude
// models the scenarios choose, each leveling its thinking from low to high.
func claudeModels() *scripted.Response {
	model := func(name string) backendtest.Map {
		return backendtest.Map{
			"name": name, "reasoning": true, "tool_call": true, "attachment": true,
			"reasoning_options": []any{backendtest.Map{"type": "effort", "values": []any{"low", "medium", "high"}}},
			"limit":             backendtest.Map{"context": 1_000_000, "output": 64_000},
		}
	}
	return served(backendtest.Map{"anthropic": backendtest.Map{
		"id": "anthropic", "name": "Anthropic", "npm": "@ai-sdk/anthropic",
		"models": backendtest.Map{"claude-opus-4-8": model("Claude Opus 4.8"), "claude-sonnet-4-6": model("Claude Sonnet 4.6")},
	}})
}

// Cost: one backend, the Cloud and a paired device, over a second: the Cloud
// boots and installs the demi.claude package, which installs the CLI and runs it.
func TestAConversationOnAPairedDeviceInfersThroughTheCloudsCLIWithTheActiveAccountsToken(t *testing.T) {
	t.Parallel()
	distribution := scripted.StartVendor(t)
	distribution.RespondAt("/releases/latest", scripted.Status(200).Chunk([]byte(claudeNewest+"\n")))
	distribution.RespondAt("/releases/"+claudeNewest+"/manifest.json", claudeManifest(claudeNewest))
	// The catalog the conversation picks its model from.
	catalog := scripted.StartVendor(t)
	catalog.RespondAt("/api.json", claudeModels())
	h := backendtest.New(t,
		backendtest.WithClaudePackage(),
		backendtest.WithClaudeReleases(distribution.URL("/releases")),
		backendtest.WithModelsDev(catalog.URL("/api.json")))
	b, master := h.StartSetUp()

	// Adding the first account starts the install on the Cloud, which has no CLI;
	// the install fails, and the account stays.
	imported := b.Post("/api/providers/setup-token", master, backendtest.Map{"token": claudeFirstToken, "label": "Claude"}).Expect(http.StatusCreated)
	provider := imported.Str("provider.id")
	failed := settledCLI(t, b, master, provider)
	if failed.Str("install.state") != "failed" {
		t.Fatalf("the install did not fail: %s", failed.Body)
	}
	reason := "Claude Code " + claudeNewest + " could not be installed: invalid release record"
	if !strings.HasPrefix(failed.Str("install.message"), reason) {
		t.Fatalf("the install failed with %q", failed.Str("install.message"))
	}
	backendtest.AssertJSON(t, failed.At("newest"), backendtest.Map{"type": "read", "version": claudeNewest})
	devices := h.Manager.Devices()
	if len(devices) != 1 {
		t.Fatalf("the install woke the user's Cloud: %v", devices)
	}
	cloud := devices[0]
	backendtest.AssertJSON(t, failed.At("machines"), []any{backendtest.Map{"deviceId": cloud, "name": "Cloud", "versions": []any{}}})
	if accounts, _ := b.Get("/api/providers/"+provider+"/accounts", master).At("accounts").([]any); len(accounts) != 1 {
		t.Fatalf("the provider has %d accounts", len(accounts))
	}

	// A request of a conversation whose files are on a paired laptop needs the CLI
	// on the Cloud: it fails the same way.
	b.CreateConversation(master, claudeConversation)
	laptop := onDeviceConversation(t, b, master, claudeConversation)
	b.Choose(master, claudeConversation, provider, "claude-opus-4-8").Expect(http.StatusOK)
	socket := b.Connect(master, claudeConversation)
	socket.Open()
	socket.Chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "hello")
	blocks := b.Transcript(master, claudeConversation)
	last := scenarioItem(t, blocks, len(blocks)-1)
	if backendtest.At(last, "type") != "error" || !strings.HasPrefix(backendtest.At(last, "message").(string), reason) {
		t.Fatalf("the request did not fail as the install did: %v", last)
	}

	// A Cloud with an older CLI answers with it; Install says so.
	home := h.Manager.Home(cloud)
	older := installByHand(t, home, claudeOlder)
	b.Post("/api/providers/"+provider+"/cli/install", master, backendtest.Map{}).Expect(http.StatusAccepted)
	installed := settledCLI(t, b, master, provider)
	backendtest.AssertJSON(t, installed.At("install"), backendtest.Map{"state": "installed", "path": older})
	backendtest.AssertJSON(t, installed.At("machines.0.versions"), []any{claudeOlder})
	// The distribution was read once: its answer is believed.
	if count := len(distribution.Requests()); count != 2 {
		t.Fatalf("the distribution was asked %d times", count)
	}

	// The request infers through the Cloud's CLI, in Demi's directories there,
	// with the account's token; nothing of it reaches the laptop.
	answer := func(token string) string {
		return "cli=" + older + " token=" + token + " cwd=" + home + "/.demi/claude/run config=" + home + "/.demi/claude/config"
	}
	socket.Chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a02", "hello again")
	if got := lastText(t, b.Transcript(master, claudeConversation)); got != answer(claudeFirstToken) {
		t.Fatalf("the CLI answered %q, want %q", got, answer(claudeFirstToken))
	}
	if _, err := os.Stat(filepath.Join(laptop.Home(), ".demi", "claude")); err == nil {
		t.Fatal("the laptop holds the CLI's directory")
	}

	// An unchanged entry keeps the same CLI process across turns. The final
	// process log below detects an unnecessary runtime restart at the wire.
	socket.Chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a04", "same account again")
	if got := lastText(t, b.Transcript(master, claudeConversation)); got != answer(claudeFirstToken) {
		t.Fatalf("the unchanged entry's CLI answered %q", got)
	}

	// After another account is selected, the next request closes the process and
	// starts one with that account's token.
	added := b.Post("/api/providers/"+provider+"/accounts", master, backendtest.Map{"token": claudeSecondToken}).Expect(http.StatusCreated)
	b.Put("/api/providers/"+provider+"/accounts/active", master, backendtest.Map{"credentialId": added.Str("account.id")}).Expect(http.StatusOK)
	socket.Chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a03", "and again")
	if got := lastText(t, b.Transcript(master, claudeConversation)); got != answer(claudeSecondToken) {
		t.Fatalf("the CLI answered %q, want %q", got, answer(claudeSecondToken))
	}
	processes, err := os.ReadFile(filepath.Join(home, "claude-processes.log"))
	if err != nil {
		t.Fatal(err)
	}
	want := "started " + claudeFirstToken + "\nended " + claudeFirstToken + "\nstarted " + claudeSecondToken + "\n"
	if string(processes) != want {
		t.Fatalf("the CLI's processes:\n%s\nwant:\n%s", processes, want)
	}
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner, about a second.
//
// backend.md § Startup and shutdown: "Claude Code CLI installs are cancelled;
// the next need starts them again", and the shutdown runs every step, so a
// backend asked to stop while an install runs exits without a failure.
// Terminate uses the harness deadline only as a guard against a hung shutdown.
func TestAShutdownCancelsARunningCLIInstall(t *testing.T) {
	t.Parallel()
	distribution := scripted.StartVendor(t)
	// The release's manifest is asked for and never answered, so the install is
	// under way for as long as the backend lives.
	held := scripted.NewGate()
	distribution.Handle(func(request scripted.Request) *scripted.Response {
		if strings.HasSuffix(request.Path, "/manifest.json") {
			return claudeManifest(claudeNewest).After(held)
		}
		return scripted.Status(200).Chunk([]byte(claudeNewest + "\n"))
	})
	catalog := scripted.StartVendor(t)
	catalog.RespondAt("/api.json", claudeModels())
	h := backendtest.New(t,
		backendtest.WithClaudePackage(),
		backendtest.WithClaudeReleases(distribution.URL("/releases")),
		backendtest.WithModelsDev(catalog.URL("/api.json")))
	b, master := h.StartSetUp()
	b.Post("/api/providers/setup-token", master, backendtest.Map{"token": claudeFirstToken, "label": "Claude"}).Expect(http.StatusCreated)
	backendtest.Eventually(t, "the install asks for the release's manifest", func() bool {
		for _, request := range distribution.Requests() {
			if strings.HasSuffix(request.Path, "/manifest.json") {
				return true
			}
		}
		return false
	})
	started := time.Now()
	code := b.Terminate()()
	if code != 0 {
		t.Fatalf("the backend exited with %d, %v after the stop signal:\n%s", code, time.Since(started), b.Logs())
	}
}

// Cost: one backend and a scripted manager; a broken shutdown costs Patience.
// The install has started a Cloud boot whose runner never connects. Shutdown
// cancels the install (backend.md § Startup and shutdown). Keep the production
// runner timeout so this scenario also exposes any shutdown wait on its boot.
// Patience is a test guard, not a documented product shutdown deadline.
func TestAShutdownDuringAnInstallsCloudBootCompletes(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t, backendtest.WithClaudePackage())
	h.Manager.Configure(func(script *scripted.Script) { script.SilentWake = true })
	b, master := h.StartSetUp()
	b.Post("/api/providers/setup-token", master, backendtest.Map{"token": claudeFirstToken, "label": "Claude"}).Expect(http.StatusCreated)
	backendtest.Eventually(t, "the install wakes the user's Cloud", func() bool {
		for _, call := range h.Manager.Calls() {
			if strings.HasPrefix(call, "wake:") {
				return true
			}
		}
		return false
	})
	started := time.Now()
	if code := b.Terminate()(); code != 0 {
		t.Fatalf("shutdown waited beyond the test guard during the install's Cloud boot: exit %d after %v\n%s", code, time.Since(started), b.Logs())
	}
	devices := h.Manager.Devices()
	if len(devices) != 1 {
		t.Fatalf("the install woke one Cloud: %v", devices)
	}
	if saves := h.Manager.Count("hibernate:" + devices[0]); saves != 1 {
		t.Fatalf("shutdown saved the Cloud %d times, want 1", saves)
	}
}
