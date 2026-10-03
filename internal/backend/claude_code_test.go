//go:build acceptance

package backend_test

import (
	"bytes"
	"context"
	"crypto/sha256"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/cmdpkg/claudecode/claudecodeop"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

const realAccountToken = "sk-ant-oat01-demi-suite-made-up-token"
const realAccountConversation = "4d2e3f5a-9b4c-4d2f-8e3a-6b2d3f4a5c01"
const realAccountModel = "claude-opus-4-8"
const realAccountOtherModel = "claude-sonnet-4-6"
const realAccountShell = "mcp__main__shell_exec"
const realAccountReleases = "/claude-code-releases"

type realAccountWorld struct {
	b                         *backendtest.TestBackend
	master                    backendtest.Session
	manager                   *backendtest.ScriptedManager
	distribution, vendor      *providertest.MockVendor
	provider, version, digest string
}

// realAccountStart installs the opt-in CLI from a local verified distribution, with all inference scripted.
func realAccountStart(ctx context.Context, t *testing.T) *realAccountWorld {
	t.Helper()
	path := os.Getenv("DEMI_TEST_CLAUDE_CODE")
	if path == "" {
		t.Skip("requires DEMI_TEST_CLAUDE_CODE and SSL_CERT_FILE naming testdata/accounts/claude_code/distribution-ca.pem")
	}
	caPath := os.Getenv("SSL_CERT_FILE")
	expected, err := os.ReadFile("testdata/accounts/claude_code/distribution-ca.pem")
	if err != nil {
		t.Fatal(err)
	}
	ca, err := os.ReadFile(caPath)
	if err != nil {
		t.Fatal(err)
	}
	accountEqual(t, ca, expected)
	executable, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	home := t.TempDir()
	command := exec.CommandContext(ctx, path, "--version")
	quiet := []string{"PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1", "DISABLE_TELEMETRY=1", "DISABLE_ERROR_REPORTING=1"}
	command.Env = append(slices.Clone(quiet), "DISABLE_AUTOUPDATER=1", "HOME="+home, "CLAUDE_CONFIG_DIR="+home)
	output, err := command.Output()
	if err != nil {
		t.Fatal(err)
	}
	fields := strings.Fields(string(output))
	if len(fields) == 0 {
		t.Fatal("CLI printed no version")
	}
	version := fields[0]
	if _, err := claudecodeop.ParseVersion(version); err != nil {
		t.Fatalf("CLI printed no version: %s", output)
	}
	digest := fmt.Sprintf("%x", sha256.Sum256(executable))
	certificate, err := os.ReadFile("testdata/accounts/claude_code/distribution.pem")
	if err != nil {
		t.Fatal(err)
	}
	key, err := os.ReadFile("testdata/accounts/claude_code/distribution.key")
	if err != nil {
		t.Fatal(err)
	}
	distribution := providertest.StartTLSVendor(t, certificate, key)
	distribution.RespondAt(realAccountReleases+"/latest", accountVendorResponse(version+"\n"))
	platforms := []string{"darwin-arm64", "darwin-x64", "linux-arm64", "linux-arm64-musl", "linux-x64", "linux-x64-musl", "win32-arm64", "win32-x64"}
	var entries []string
	build := fmt.Sprintf(`{"binary":"claude","checksum":"%s","size":%d}`, digest, len(executable))
	for _, platform := range platforms {
		entries = append(entries, `"`+platform+`":`+build)
		distribution.RespondAt(realAccountReleases+"/"+version+"/"+platform+"/claude", providertest.MockResponse{Status: 200, Chunks: [][]byte{executable}})
	}
	distribution.RespondAt(realAccountReleases+"/"+version+"/manifest.json", accountVendorResponse(`{"version":"`+version+`","platforms":{`+strings.Join(entries, ",")+`}}`))
	vendor := providertest.StartVendor(t)
	vendor.RespondAt("/api.json", accountClaudeModels())
	h, manager := accountClaudeHarness(ctx, t)
	h.Config.ClaudeReleases, err = url.Parse(distribution.URL(realAccountReleases))
	if err != nil {
		t.Fatal(err)
	}
	h.Config.ModelsDevURL, err = url.Parse(vendor.URL("/api.json"))
	if err != nil {
		t.Fatal(err)
	}
	manager.SetScript(backendtest.MachineScript{CloudEnv: map[string]string{"PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1", "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1", "ANTHROPIC_BASE_URL": vendor.URL(""), "SSL_CERT_FILE": caPath}})
	b, master := accountStart(ctx, t, h)
	imported := accountRequest(ctx, t, b, "POST", "/api/providers/setup-token", &master, `{"token":"`+realAccountToken+`","label":"Claude"}`)
	accountEqual(t, imported.Status, 201)
	id := string(accountDecode(t, imported, webapi.DecodeProviderAnswer).Provider.ID)
	installed := accountCLISettled(ctx, t, b, &master, id)
	if _, ok := installed.Install.(*webapi.CLIInstallInstalled); !ok {
		t.Fatalf("Cloud did not install CLI: %#v", installed)
	}
	return &realAccountWorld{b: b, master: master, manager: manager, distribution: distribution, vendor: vendor, provider: id, version: version, digest: digest}
}

// conversation opens the model through the real backend and its page socket.
func (w *realAccountWorld) conversation(ctx context.Context, t *testing.T) *backendtest.AccountSocket {
	t.Helper()
	accountEqual(t, accountRequest(ctx, t, w.b, "POST", "/api/conversations", &w.master, `{"id":"`+realAccountConversation+`"}`).Status, 201)
	accountEqual(t, accountRequest(ctx, t, w.b, "PATCH", "/api/conversations/"+realAccountConversation, &w.master, `{"model":{"providerId":"`+w.provider+`","modelId":"`+realAccountModel+`"}}`).Status, 200)
	socket, err := backendtest.OpenAccountSocket(ctx, t, w.b, &w.master, realAccountConversation)
	if err != nil {
		t.Fatal(err)
	}
	if err := socket.Open(ctx); err != nil {
		t.Fatal(err)
	}
	return socket
}
func (w *realAccountWorld) blocks(ctx context.Context, t *testing.T) []core.Block {
	return accountTranscript(ctx, t, w.b, &w.master, realAccountConversation)
}
func (w *realAccountWorld) inferences() []providertest.RecordedRequest {
	var result []providertest.RecordedRequest
	for _, r := range w.vendor.Requests() {
		if r.Method == "POST" && r.URI == "/v1/messages" {
			result = append(result, r)
		}
	}
	return result
}
func (w *realAccountWorld) answers(response providertest.MockResponse) {
	w.vendor.RespondAt("/v1/messages", response)
}
func (w *realAccountWorld) usage(ctx context.Context, t *testing.T) []webapi.UsageGroup {
	return accountDecode(t, accountRequest(ctx, t, w.b, "GET", "/api/usage", &w.master, ""), webapi.DecodeUsageTotals).Totals
}

// realAccountString encodes text placed in a scripted vendor frame.
func realAccountString(t *testing.T, text string) string {
	t.Helper()
	data, err := contract.EncodeJSON(text)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// realAccountEvent emits one Messages API event with its typed name.
func realAccountEvent(name, body string) string { return "event: " + name + "\ndata: " + body + "\n\n" }

// realAccountStartMessage begins the scripted vendor's response.
func realAccountStartMessage(usage string) string {
	return realAccountEvent("message_start", `{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-opus-4-8","content":[],"usage":`+usage+`}}`)
}

// realAccountText streams each text delta independently.
func realAccountText(t *testing.T, index int, texts ...string) string {
	result := realAccountEvent("content_block_start", fmt.Sprintf(`{"type":"content_block_start","index":%d,"content_block":{"type":"text","text":""}}`, index))
	for _, text := range texts {
		result += realAccountEvent("content_block_delta", fmt.Sprintf(`{"type":"content_block_delta","index":%d,"delta":{"type":"text_delta","text":%s}}`, index, realAccountString(t, text)))
	}
	return result + realAccountEvent("content_block_stop", fmt.Sprintf(`{"type":"content_block_stop","index":%d}`, index))
}

// realAccountThinking streams reasoning and its replay signature.
func realAccountThinking(t *testing.T, index int, text, signature string) string {
	return realAccountEvent("content_block_start", fmt.Sprintf(`{"type":"content_block_start","index":%d,"content_block":{"type":"thinking","thinking":"","signature":""}}`, index)) + realAccountEvent("content_block_delta", fmt.Sprintf(`{"type":"content_block_delta","index":%d,"delta":{"type":"thinking_delta","thinking":%s}}`, index, realAccountString(t, text))) + realAccountEvent("content_block_delta", fmt.Sprintf(`{"type":"content_block_delta","index":%d,"delta":{"type":"signature_delta","signature":%s}}`, index, realAccountString(t, signature))) + realAccountEvent("content_block_stop", fmt.Sprintf(`{"type":"content_block_stop","index":%d}`, index))
}

// realAccountTool asks the CLI to run one tool through Demi's MCP server.
func realAccountTool(t *testing.T, index int, id, printed string) string {
	input := `{"description":"Run","script":"printf '` + printed + `'","timeoutMs":60000}`
	return realAccountEvent("content_block_start", fmt.Sprintf(`{"type":"content_block_start","index":%d,"content_block":{"type":"tool_use","id":"%s","name":"%s","input":{}}}`, index, id, realAccountShell)) + realAccountEvent("content_block_delta", fmt.Sprintf(`{"type":"content_block_delta","index":%d,"delta":{"type":"input_json_delta","partial_json":%s}}`, index, realAccountString(t, input))) + realAccountEvent("content_block_stop", fmt.Sprintf(`{"type":"content_block_stop","index":%d}`, index))
}

// realAccountMessage completes one scripted response, with the reported usage.
func realAccountMessage(blocks, stop, usage string, output uint64) providertest.MockResponse {
	return providertest.EventStream(realAccountStartMessage(usage) + blocks + realAccountEvent("message_delta", fmt.Sprintf(`{"type":"message_delta","delta":{"stop_reason":"%s"},"usage":{"output_tokens":%d}}`, stop, output)) + realAccountEvent("message_stop", `{"type":"message_stop"}`))
}

// realAccountKinds observes the serialized block types without duplicating the union.
func realAccountKinds(t *testing.T, blocks []core.Block) []string {
	t.Helper()
	kinds := []string{}
	for _, block := range blocks {
		data, err := contract.EncodeJSON(block)
		if err != nil {
			t.Fatal(err)
		}
		fields, err := contract.ObjectFields(data)
		if err != nil {
			t.Fatal(err)
		}
		for _, field := range fields {
			if field.Name == "type" {
				encoded, err := contract.EncodeJSON(field.Value)
				if err != nil {
					t.Fatal(err)
				}
				kind, err := contract.Decode[string](encoded)
				if err != nil {
					t.Fatal(err)
				}
				kinds = append(kinds, kind)
			}
		}
	}
	return kinds
}

// realAccountMessages observes user and assistant texts carried to the scripted vendor.
func realAccountMessages(t *testing.T, r providertest.RecordedRequest) [][2]any {
	t.Helper()
	body, ok := r.JSON(t).(map[string]any)
	if !ok {
		t.Fatal("vendor request is not an object")
	}
	messages, ok := body["messages"].([]any)
	if !ok {
		t.Fatal("vendor request has no messages")
	}
	var result [][2]any
	for _, value := range messages {
		m, ok := value.(map[string]any)
		if !ok {
			t.Fatal("message is not an object")
		}
		if m["role"] == "system" {
			continue
		}
		texts := []string{}
		switch content := m["content"].(type) {
		case string:
			texts = append(texts, content)
		case []any:
			for _, value := range content {
				if block, ok := value.(map[string]any); ok {
					if text, ok := block["text"].(string); ok {
						texts = append(texts, text)
					}
				}
			}
		}
		result = append(result, [2]any{m["role"], texts})
	}
	return result
}

// realAccountReplayed checks the full ordered transcript sent to a new CLI process.
func realAccountReplayed(t *testing.T, r providertest.RecordedRequest, parts ...string) {
	t.Helper()
	messages := realAccountMessages(t, r)
	accountEqual(t, len(messages), 1)
	accountEqual(t, messages[0][0], any("user"))
	texts, ok := messages[0][1].([]string)
	if !ok {
		t.Fatal(messages)
	}
	text := strings.Join(texts, "\n")
	for _, part := range parts {
		at := strings.Index(text, part)
		if at < 0 {
			t.Fatalf("replay lacks %q in order: %s", part, text)
		}
		text = text[at+len(part):]
	}
}

// realAccountChat drives a turn to durable completion.
func realAccountChat(ctx context.Context, t *testing.T, s *backendtest.AccountSocket, id, text string) {
	t.Helper()
	if _, err := s.Chat(ctx, id, text); err != nil {
		t.Fatal(err)
	}
}

// Several seconds: each Cloud installs and verifies the supplied CLI, whose processes start for real.
func TestTheCloudsVerifiedCLIStreamsReasoningAndTextAndRunsAToolBatchThroughDemiInOneProcess(t *testing.T) {
	ctx := t.Context()
	w := realAccountStart(ctx, t)
	devices := w.manager.Devices()
	accountEqual(t, len(devices), 1)
	home := w.manager.Home(devices[0])
	cli := accountCLISettled(ctx, t, w.b, &w.master, w.provider)
	path := filepath.Join(home, ".demi/claude", w.version, "claude")
	accountEqual[webapi.CLIInstall](t, cli.Install, &webapi.CLIInstallInstalled{Path: path})
	accountEqual[webapi.NewestVersion](t, cli.Newest, &webapi.NewestVersionRead{Version: w.version})
	accountEqual(t, *cli.Machines[0].Versions, []string{w.version})
	installed, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	accountEqual(t, fmt.Sprintf("%x", sha256.Sum256(installed)), w.digest)
	read := w.distribution.Requests()
	accountEqual(t, len(read), 3)
	accountEqual(t, read[0].URI, realAccountReleases+"/latest")
	accountEqual(t, read[1].URI, realAccountReleases+"/"+w.version+"/manifest.json")
	if !strings.HasPrefix(read[2].URI, realAccountReleases+"/"+w.version+"/") {
		t.Fatal(read[2].URI)
	}
	socket := w.conversation(ctx, t)
	w.answers(realAccountMessage(realAccountThinking(t, 0, "Weighing a greeting.", "signature-1")+realAccountText(t, 1, "Hello", " from the vendor."), "end_turn", `{"input_tokens":11,"output_tokens":1,"cache_read_input_tokens":5,"cache_creation_input_tokens":3}`, 7))
	realAccountChat(ctx, t, socket, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a11", "Say hello.")
	blocks := w.blocks(ctx, t)
	accountEqual(t, realAccountKinds(t, blocks), []string{"user", "thinking", "text", "response"})
	thinking, ok := blocks[1].(*core.ThinkingBlock)
	if !ok {
		t.Fatal(blocks[1])
	}
	accountEqual(t, thinking.Text, "Weighing a greeting.")
	accountEqual(t, *thinking.Signature, "signature-1")
	accountEqual(t, accountLastText(blocks), "Hello from the vendor.")
	response, ok := blocks[3].(*core.ResponseBlock)
	if !ok {
		t.Fatal(blocks[3])
	}
	accountEqual(t, response.Usage, core.TokenUsage{InputTokens: 11, OutputTokens: 7, CacheReadTokens: 5, CacheWriteTokens: 3})
	totals := w.usage(ctx, t)
	accountEqual(t, len(totals), 1)
	row := totals[0]
	accountEqual(t, []uint64{row.Requests, row.InputTokens, row.OutputTokens, row.CacheReadTokens, row.CacheWriteTokens}, []uint64{1, 11, 7, 5, 3})
	inferences := w.inferences()
	accountEqual(t, len(inferences), 1)
	first := inferences[0]
	accountEqual(t, first.Header("authorization"), "Bearer "+realAccountToken)
	body, ok := first.JSON(t).(map[string]any)
	if !ok {
		t.Fatal("request object")
	}
	accountEqual(t, body["model"], any(realAccountModel))
	system, err := contract.EncodeJSON(body["system"])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(system, []byte("You are a coding agent")) {
		t.Fatal(string(system))
	}
	tools, ok := body["tools"].([]any)
	if !ok {
		t.Fatal("tools missing")
	}
	hasShell := false
	for _, value := range tools {
		if tool, ok := value.(map[string]any); ok && tool["name"] == realAccountShell {
			hasShell = true
		}
	}
	accountEqual(t, hasShell, true)
	accountEqual(t, realAccountMessages(t, first), [][2]any{{"user", []string{"Say hello."}}})
	w.answers(realAccountMessage(realAccountTool(t, 0, "toolu_suite_1", "the first ran on the Cloud")+realAccountTool(t, 1, "toolu_suite_2", "the second ran on the Cloud"), "tool_use", `{"input_tokens":20,"output_tokens":1}`, 9))
	w.answers(realAccountMessage(realAccountText(t, 0, "Both ran."), "end_turn", `{"input_tokens":30,"output_tokens":1}`, 4))
	realAccountChat(ctx, t, socket, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a12", "Run the tools.")
	blocks = w.blocks(ctx, t)
	accountEqual(t, realAccountKinds(t, blocks[4:]), []string{"user", "tool_call", "tool_call", "text", "response"})
	for i, printed := range []string{"the first ran on the Cloud", "the second ran on the Cloud"} {
		ran, ok := blocks[5+i].(*core.ToolCallBlock)
		if !ok {
			t.Fatal(blocks[5+i])
		}
		accountEqual(t, ran.ToolUseID, fmt.Sprintf("toolu_suite_%d", i+1))
		accountEqual(t, ran.ToolName, "shell_exec")
		accountEqual(t, ran.Status, core.ToolCallStatusCompleted)
		var text string
		for _, part := range ran.Output {
			if p, ok := part.(*core.ToolText); ok {
				text += p.Text
			}
		}
		if !strings.Contains(text, printed) {
			t.Fatal(text)
		}
	}
	accountEqual(t, accountLastText(blocks), "Both ran.")
	response, ok = blocks[8].(*core.ResponseBlock)
	if !ok {
		t.Fatal(blocks[8])
	}
	accountEqual(t, []uint64{response.Usage.InputTokens, response.Usage.OutputTokens}, []uint64{50, 13})
	inferences = w.inferences()
	accountEqual(t, len(inferences), 3)
	first, called, answered := inferences[0], inferences[1], inferences[2]
	accountEqual(t, realAccountProcess(t, called), realAccountProcess(t, first))
	accountEqual(t, realAccountProcess(t, answered), realAccountProcess(t, first))
	continued := realAccountMessages(t, called)
	accountEqual(t, len(continued), 3)
	accountEqual(t, continued[2], [2]any{"user", []string{"Run the tools."}})
	resultBody, ok := answered.JSON(t).(map[string]any)
	if !ok {
		t.Fatal("request object")
	}
	messages, ok := resultBody["messages"].([]any)
	if !ok {
		t.Fatal("messages missing")
	}
	var results []map[string]any
	for _, value := range messages {
		if m, ok := value.(map[string]any); ok {
			if content, ok := m["content"].([]any); ok {
				for _, value := range content {
					if block, ok := value.(map[string]any); ok && block["type"] == "tool_result" {
						results = append(results, block)
					}
				}
			}
		}
	}
	accountEqual(t, len(results), 2)
	for i, printed := range []string{"the first ran on the Cloud", "the second ran on the Cloud"} {
		accountEqual(t, results[i]["tool_use_id"], any(fmt.Sprintf("toolu_suite_%d", i+1)))
		encoded, err := contract.EncodeJSON(results[i])
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Contains(encoded, []byte(printed)) {
			t.Fatal(string(encoded))
		}
	}
}

// Several seconds: the real CLI is installed and restarted after stop and model changes.
func TestStopEndsTheCLIsStreamAndEachNewProcessReplaysTheTranscriptForItsModelAndEffort(t *testing.T) {
	ctx := t.Context()
	w := realAccountStart(ctx, t)
	socket := w.conversation(ctx, t)
	stream := providertest.EventStream(realAccountStartMessage(`{"input_tokens":12,"output_tokens":1}`) + realAccountText(t, 0, "The long answer begins"))
	stream.Chunks[0] = bytes.TrimSuffix(stream.Chunks[0], []byte(realAccountEvent("content_block_stop", `{"type":"content_block_stop","index":0}`)))
	stream.Ending = providertest.Open
	w.answers(stream)
	if err := socket.Send(ctx, &framewire.SendFrame{MessageID: "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a21", Content: []framewire.ClientContent{&framewire.TextContent{Text: "Write a long answer."}}}); err != nil {
		t.Fatal(err)
	}
	for {
		frame, err := socket.Next(ctx)
		if err != nil {
			t.Fatal(err)
		}
		encoded, err := contract.EncodeJSON(frame)
		if err != nil {
			t.Fatal(err)
		}
		if bytes.Contains(encoded, []byte("The long answer begins")) {
			break
		}
	}
	if err := socket.Send(ctx, &framewire.AbortFrame{}); err != nil {
		t.Fatal(err)
	}
	answered, idle := false, false
	for !answered || !idle {
		frame, err := socket.Next(ctx)
		if err != nil {
			t.Fatal(err)
		}
		if _, ok := frame.(*framewire.AbortResultFrame); ok {
			answered = true
		}
		if phase, ok := frame.(*framewire.PhaseFrame); ok && phase.Phase == core.SessionPhaseIdle {
			idle = true
		}
	}
	w.vendor.Disconnected(ctx)
	accountEqual(t, accountLastText(w.blocks(ctx, t)), "The long answer begins")
	w.answers(realAccountMessage(realAccountText(t, 0, "A short answer."), "end_turn", `{"input_tokens":13,"output_tokens":1}`, 3))
	realAccountChat(ctx, t, socket, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a22", "Answer briefly instead.")
	accountEqual(t, accountLastText(w.blocks(ctx, t)), "A short answer.")
	inferences := w.inferences()
	accountEqual(t, len(inferences), 2)
	if realAccountProcess(t, inferences[0]) == realAccountProcess(t, inferences[1]) {
		t.Fatal("stop kept CLI process")
	}
	realAccountReplayed(t, inferences[1], "Write a long answer.", "The long answer begins", "Answer briefly instead.")
	accountEqual(t, accountRequest(ctx, t, w.b, "PATCH", "/api/conversations/"+realAccountConversation, &w.master, `{"model":{"providerId":"`+w.provider+`","modelId":"`+realAccountOtherModel+`"},"thinkingEffort":"medium"}`).Status, 200)
	w.answers(realAccountMessage(realAccountText(t, 0, "Another model answers."), "end_turn", `{"input_tokens":14,"output_tokens":1}`, 3))
	realAccountChat(ctx, t, socket, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a23", "Once more.")
	accountEqual(t, accountLastText(w.blocks(ctx, t)), "Another model answers.")
	inferences = w.inferences()
	accountEqual(t, len(inferences), 3)
	if realAccountProcess(t, inferences[1]) == realAccountProcess(t, inferences[2]) {
		t.Fatal("model change kept CLI process")
	}
	body, ok := inferences[2].JSON(t).(map[string]any)
	if !ok {
		t.Fatal("request object")
	}
	accountEqual(t, body["model"], any(realAccountOtherModel))
	config, ok := body["output_config"].(map[string]any)
	if !ok {
		t.Fatal("output_config missing")
	}
	accountEqual(t, config["effort"], any("medium"))
	realAccountReplayed(t, inferences[2], "Write a long answer.", "The long answer begins", "Answer briefly instead.", "A short answer.", "Once more.")
}

// Several seconds: the real CLI is installed and handles two scripted vendor requests.
func TestAVendorErrorFailsTheRequestInTheCLIsWordsAndTheKeptProcessAnswersTheNext(t *testing.T) {
	ctx := t.Context()
	w := realAccountStart(ctx, t)
	socket := w.conversation(ctx, t)
	refusal := accountVendorResponse(`{"type":"error","error":{"type":"invalid_request_error","message":"The scripted vendor refuses this request."}}`)
	refusal.Status = 400
	refusal.Headers = http.Header{"Content-Type": []string{"application/json"}}
	w.answers(refusal)
	realAccountChat(ctx, t, socket, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a31", "Hello?")
	blocks := w.blocks(ctx, t)
	accountEqual(t, realAccountKinds(t, blocks), []string{"user", "error"})
	failed, ok := blocks[1].(*core.ErrorBlock)
	if !ok {
		t.Fatal(blocks[1])
	}
	if !strings.Contains(failed.Message, "The scripted vendor refuses this request.") {
		t.Fatal(failed.Message)
	}
	accountEqual(t, len(w.usage(ctx, t)), 0)
	w.answers(realAccountMessage(realAccountText(t, 0, "Better now."), "end_turn", `{"input_tokens":8,"output_tokens":1}`, 2))
	realAccountChat(ctx, t, socket, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a32", "Try again.")
	blocks = w.blocks(ctx, t)
	accountEqual(t, realAccountKinds(t, blocks[2:]), []string{"user", "text", "response"})
	accountEqual(t, accountLastText(blocks), "Better now.")
	inferences := w.inferences()
	accountEqual(t, len(inferences), 2)
	accountEqual(t, realAccountProcess(t, inferences[1]), realAccountProcess(t, inferences[0]))
	accountEqual(t, w.usage(ctx, t)[0].Requests, uint64(1))
}

// realAccountProcess observes the CLI session identity the vendor received.
func realAccountProcess(t *testing.T, request providertest.RecordedRequest) string {
	t.Helper()
	id := request.Header("x-claude-code-session-id")
	if id == "" {
		t.Fatal("CLI did not name its session")
	}
	return id
}
