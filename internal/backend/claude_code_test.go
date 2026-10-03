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

const (
	realAccountToken        = "sk-ant-oat01-demi-suite-made-up-token"
	realAccountConversation = "4d2e3f5a-9b4c-4d2f-8e3a-6b2d3f4a5c01"
	realAccountModel        = "claude-opus-4-8"
	realAccountOtherModel   = "claude-sonnet-4-6"
	realAccountShell        = "mcp__main__shell_exec"
	realAccountReleases     = "/claude-code-releases"
)

type realAccountWorld struct {
	server                    *backendtest.TestBackend
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
		t.Skip(
			"requires DEMI_TEST_CLAUDE_CODE and SSL_CERT_FILE naming testdata/accounts/claude_code/distribution-ca.pem",
		)
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
	conversationEqual(t, ca, expected)
	executable, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	home := t.TempDir()
	command := exec.CommandContext(ctx, path, "--version")
	quiet := []string{
		"PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
		"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1",
		"DISABLE_TELEMETRY=1",
		"DISABLE_ERROR_REPORTING=1",
	}
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
	platforms := []string{
		"darwin-arm64",
		"darwin-x64",
		"linux-arm64",
		"linux-arm64-musl",
		"linux-x64",
		"linux-x64-musl",
		"win32-arm64",
		"win32-x64",
	}
	var entries []string
	build := fmt.Sprintf(`{"binary":"claude","checksum":"%s","size":%d}`, digest, len(executable))
	for _, platform := range platforms {
		entries = append(entries, `"`+platform+`":`+build)
		distribution.RespondAt(
			realAccountReleases+"/"+version+"/"+platform+"/claude",
			providertest.MockResponse{Status: 200, Chunks: [][]byte{executable}},
		)
	}
	distribution.RespondAt(
		realAccountReleases+"/"+version+"/manifest.json",
		accountVendorResponse(`{"version":"`+version+`","platforms":{`+strings.Join(entries, ",")+`}}`),
	)
	vendor := providertest.StartVendor(t)
	vendor.RespondAt("/api.json", accountClaudeModels())
	harness, manager := accountClaudeHarness(ctx, t)
	harness.Config.ClaudeReleases, err = url.Parse(distribution.URL(realAccountReleases))
	if err != nil {
		t.Fatal(err)
	}
	harness.Config.ModelsDevURL, err = url.Parse(vendor.URL("/api.json"))
	if err != nil {
		t.Fatal(err)
	}
	manager.SetScript(
		backendtest.MachineScript{
			CloudEnv: map[string]string{
				"PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
				"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
				"DISABLE_TELEMETRY":                        "1",
				"DISABLE_ERROR_REPORTING":                  "1",
				"ANTHROPIC_BASE_URL":                       vendor.URL(""),
				"SSL_CERT_FILE":                            caPath,
			},
		},
	)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	imported := conversationRequest(
		ctx,
		t,
		server,
		&master,
		"POST",
		"/api/providers/setup-token",
		`{"token":"`+realAccountToken+`","label":"Claude"}`,
		201,
	)
	id := string(conversationDecode(t, imported, webapi.DecodeProviderAnswer).Provider.ID)
	installed := accountCLISettled(ctx, t, server, &master, id)
	if _, ok := installed.Install.(*webapi.CLIInstallInstalled); !ok {
		t.Fatalf("Cloud did not install CLI: %#v", installed)
	}
	return &realAccountWorld{
		server:       server,
		master:       master,
		manager:      manager,
		distribution: distribution,
		vendor:       vendor,
		provider:     id,
		version:      version,
		digest:       digest,
	}
}

// conversation opens the model through the real backend and its page socket.
func (w *realAccountWorld) conversation(ctx context.Context, t *testing.T) *backendtest.ConversationSocket {
	t.Helper()
	conversationCreate(ctx, t, w.server, &w.master, realAccountConversation)
	conversationChoose(ctx, t, w.server, &w.master, realAccountConversation, w.provider, realAccountModel)
	return conversationOpen(ctx, t, w.server, &w.master, realAccountConversation)
}

func (w *realAccountWorld) inferences(t *testing.T) []providertest.RecordedRequest {
	var result []providertest.RecordedRequest
	for _, request := range w.vendor.Requests() {
		path, err := url.ParseRequestURI(request.URI)
		wireMust(t, err)
		if request.Method == "POST" && path.Path == "/v1/messages" {
			result = append(result, request)
		}
	}
	return result
}

func (w *realAccountWorld) usage(ctx context.Context, t *testing.T) []webapi.UsageGroup {
	return conversationDecode(
		t,
		conversationRequest(ctx, t, w.server, &w.master, "GET", "/api/usage", "", 200),
		webapi.DecodeUsageTotals,
	).Totals
}

// realAccountTool asks the CLI to run one tool through Demi's MCP server.
func realAccountTool(t *testing.T, index int, id, printed string) string {
	input := `{"description":"Run","script":"printf '` + printed + `'","timeoutMs":60000}`
	return anthropicEvent(
		"content_block_start",
		fmt.Sprintf(
			`{"type":"content_block_start","index":%d,"content_block":{"type":"tool_use","id":"%s",`+
				`"name":"%s","input":{}}}`,
			index,
			id,
			realAccountShell,
		),
	) + anthropicEvent(
		"content_block_delta",
		fmt.Sprintf(
			`{"type":"content_block_delta","index":%d,"delta":{"type":"input_json_delta",`+
				`"partial_json":%s}}`,
			index,
			conversationJSON(t, input),
		),
	) + anthropicEvent(
		"content_block_stop",
		fmt.Sprintf(`{"type":"content_block_stop","index":%d}`, index),
	)
}

// conversationMessages observes user and assistant texts carried to the scripted vendor.
func conversationMessages(t *testing.T, request providertest.RecordedRequest) [][2]any {
	t.Helper()
	body, ok := request.JSON(t).(map[string]any)
	if !ok {
		t.Fatal("vendor request is not an object")
	}
	messages, ok := body["messages"].([]any)
	if !ok {
		t.Fatal("vendor request has no messages")
	}
	var result [][2]any
	for _, value := range messages {
		message, ok := value.(map[string]any)
		if !ok {
			t.Fatal("message is not an object")
		}
		if message["role"] == "system" {
			continue
		}
		texts := []string{}
		switch content := message["content"].(type) {
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
		result = append(result, [2]any{message["role"], texts})
	}
	return result
}

// realAccountReplayed checks the full ordered transcript sent to a new CLI process.
func realAccountReplayed(t *testing.T, request providertest.RecordedRequest, parts ...string) {
	t.Helper()
	messages := conversationMessages(t, request)
	conversationEqual(t, len(messages), 1)
	conversationEqual(t, messages[0][0], any("user"))
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

// TestTheCloudsVerifiedCLIStreamsReasoningAndTextAndRunsAToolBatchThroughDemiInOneProcess
// checks real CLI streaming and tool execution.
// Several seconds: each Cloud installs and verifies the supplied CLI, whose processes start for real.
func TestTheCloudsVerifiedCLIStreamsReasoningAndTextAndRunsAToolBatchThroughDemiInOneProcess(t *testing.T) {
	t.Parallel()
	ctx := t.Context()
	w := realAccountStart(ctx, t)
	devices := w.manager.Devices()
	conversationEqual(t, len(devices), 1)
	cli := accountCLISettled(ctx, t, w.server, &w.master, w.provider)
	// The ignored Rust scenario predates 700143f3a: ensure now returns the
	// verified artifact cache entry, not a versioned executable under home.
	path := filepath.Join(w.manager.State(devices[0]), "artifacts", w.digest)
	conversationEqual[webapi.CLIInstall](t, cli.Install, &webapi.CLIInstallInstalled{Path: path})
	conversationEqual[webapi.NewestVersion](t, cli.Newest, &webapi.NewestVersionRead{Version: w.version})
	conversationEqual(t, *cli.Machines[0].Versions, []string{w.version})
	installed, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	conversationEqual(t, fmt.Sprintf("%x", sha256.Sum256(installed)), w.digest)
	read := w.distribution.Requests()
	conversationEqual(t, len(read), 3)
	conversationEqual(t, read[0].URI, realAccountReleases+"/latest")
	conversationEqual(t, read[1].URI, realAccountReleases+"/"+w.version+"/manifest.json")
	if !strings.HasPrefix(read[2].URI, realAccountReleases+"/"+w.version+"/") {
		t.Fatal(read[2].URI)
	}
	socket := w.conversation(ctx, t)
	w.vendor.RespondAt(
		"/v1/messages",
		conversationMessage(
			conversationThinkingBlock(
				t,
				0,
				"Weighing a greeting.",
				"signature-1",
			)+conversationTextBlock(
				t,
				1,
				"Hello",
				" from the vendor.",
			),
			"end_turn",
			`{"input_tokens":11,"output_tokens":1,"cache_read_input_tokens":5,`+
				`"cache_creation_input_tokens":3}`,
			7,
		),
	)
	if _, err := socket.Chat(ctx, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a11", "Say hello."); err != nil {
		t.Fatal(err)
	}
	blocks := conversationTranscript(ctx, t, w.server, &w.master, realAccountConversation).Blocks
	conversationEqual(t, conversationKinds(t, blocks), []string{"user", "thinking", "text", "response"})
	thinking, ok := blocks[1].(*core.ThinkingBlock)
	if !ok {
		t.Fatal(blocks[1])
	}
	conversationEqual(t, thinking.Text, "Weighing a greeting.")
	conversationEqual(t, *thinking.Signature, "signature-1")
	conversationEqual(t, conversationLastText(t, blocks), "Hello from the vendor.")
	response, ok := blocks[3].(*core.ResponseBlock)
	if !ok {
		t.Fatal(blocks[3])
	}
	conversationEqual(
		t,
		response.Usage,
		core.TokenUsage{InputTokens: 11, OutputTokens: 7, CacheReadTokens: 5, CacheWriteTokens: 3},
	)
	totals := w.usage(ctx, t)
	conversationEqual(t, len(totals), 1)
	row := totals[0]
	conversationEqual(
		t,
		[]uint64{row.Requests, row.InputTokens, row.OutputTokens, row.CacheReadTokens, row.CacheWriteTokens},
		[]uint64{1, 11, 7, 5, 3},
	)
	inferences := w.inferences(t)
	conversationEqual(t, len(inferences), 1)
	first := inferences[0]
	conversationEqual(t, first.Header("authorization"), "Bearer "+realAccountToken)
	body, ok := first.JSON(t).(map[string]any)
	if !ok {
		t.Fatal("request object")
	}
	conversationEqual(t, body["model"], any(realAccountModel))
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
	conversationEqual(t, hasShell, true)
	sentMessages := conversationMessages(t, first)
	conversationEqual(t, len(sentMessages), 1)
	texts, ok := sentMessages[0][1].([]string)
	if !ok {
		t.Fatal(sentMessages)
	}
	// Unlike the Rust assertion, exclude the CLI's own leading context blocks:
	// this CLI adds a date reminder, while the scenario protects Demi's message.
	// Stop at the first other block and require the entire remainder exactly.
	for len(texts) > 0 && strings.HasPrefix(texts[0], "<system-reminder>") &&
		strings.HasSuffix(strings.TrimSpace(texts[0]), "</system-reminder>") {
		texts = texts[1:]
	}
	sentMessages[0][1] = texts
	conversationEqual(t, sentMessages, [][2]any{{"user", []string{"Say hello."}}})
	w.vendor.RespondAt(
		"/v1/messages",
		conversationMessage(
			realAccountTool(
				t,
				0,
				"toolu_suite_1",
				"the first ran on the Cloud",
			)+realAccountTool(
				t,
				1,
				"toolu_suite_2",
				"the second ran on the Cloud",
			),
			"tool_use",
			`{"input_tokens":20,"output_tokens":1}`,
			9,
		),
	)
	w.vendor.RespondAt(
		"/v1/messages",
		conversationMessage(
			conversationTextBlock(t, 0, "Both ran."),
			"end_turn",
			`{"input_tokens":30,"output_tokens":1}`,
			4,
		),
	)
	if _, err := socket.Chat(ctx, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a12", "Run the tools."); err != nil {
		t.Fatal(err)
	}
	blocks = conversationTranscript(ctx, t, w.server, &w.master, realAccountConversation).Blocks
	conversationEqual(
		t,
		conversationKinds(t, blocks[4:]),
		[]string{"user", "tool_call", "tool_call", "text", "response"},
	)
	for i, printed := range []string{"the first ran on the Cloud", "the second ran on the Cloud"} {
		ran, ok := blocks[5+i].(*core.ToolCallBlock)
		if !ok {
			t.Fatal(blocks[5+i])
		}
		conversationEqual(t, ran.ToolUseID, fmt.Sprintf("toolu_suite_%d", i+1))
		conversationEqual(t, ran.ToolName, "shell_exec")
		conversationEqual(t, ran.Status, core.ToolCallStatusCompleted)
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
	conversationEqual(t, conversationLastText(t, blocks), "Both ran.")
	response, ok = blocks[8].(*core.ResponseBlock)
	if !ok {
		t.Fatal(blocks[8])
	}
	conversationEqual(t, []uint64{response.Usage.InputTokens, response.Usage.OutputTokens}, []uint64{50, 13})
	inferences = w.inferences(t)
	conversationEqual(t, len(inferences), 3)
	first, called, answered := inferences[0], inferences[1], inferences[2]
	conversationEqual(t, realAccountProcess(t, called), realAccountProcess(t, first))
	conversationEqual(t, realAccountProcess(t, answered), realAccountProcess(t, first))
	continued := conversationMessages(t, called)
	conversationEqual(t, len(continued), 3)
	conversationEqual(t, continued[2], [2]any{"user", []string{"Run the tools."}})
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
		if message, ok := value.(map[string]any); ok {
			if content, ok := message["content"].([]any); ok {
				for _, value := range content {
					if block, ok := value.(map[string]any); ok && block["type"] == "tool_result" {
						results = append(results, block)
					}
				}
			}
		}
	}
	conversationEqual(t, len(results), 2)
	for i, printed := range []string{"the first ran on the Cloud", "the second ran on the Cloud"} {
		conversationEqual(t, results[i]["tool_use_id"], any(fmt.Sprintf("toolu_suite_%d", i+1)))
		encoded, err := contract.EncodeJSON(results[i])
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Contains(encoded, []byte(printed)) {
			t.Fatal(string(encoded))
		}
	}
}

// TestStopEndsTheCLIsStreamAndEachNewProcessReplaysTheTranscriptForItsModelAndEffort
// checks CLI restart and transcript replay.
// Several seconds: the real CLI is installed and restarted after stop and model changes.
func TestStopEndsTheCLIsStreamAndEachNewProcessReplaysTheTranscriptForItsModelAndEffort(t *testing.T) {
	t.Parallel()
	ctx := t.Context()
	w := realAccountStart(ctx, t)
	socket := w.conversation(ctx, t)
	stream := providertest.EventStream(
		conversationMessageStart(
			`{"input_tokens":12,"output_tokens":1}`,
		) + conversationTextBlock(
			t,
			0,
			"The long answer begins",
		),
	)
	stream.Ending = providertest.Open
	w.vendor.RespondAt("/v1/messages", stream)
	if err := socket.Send(
		ctx,
		&framewire.SendFrame{
			MessageID: "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a21",
			Content:   []framewire.ClientContent{&framewire.TextContent{Text: "Write a long answer."}},
		},
	); err != nil {
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
	wireMust(t, socket.Stop(ctx))
	w.vendor.Disconnected(ctx)
	conversationEqual(
		t,
		conversationLastText(t, conversationTranscript(ctx, t, w.server, &w.master, realAccountConversation).Blocks),
		"The long answer begins",
	)
	w.vendor.RespondAt(
		"/v1/messages",
		conversationMessage(
			conversationTextBlock(t, 0, "A short answer."),
			"end_turn",
			`{"input_tokens":13,"output_tokens":1}`,
			3,
		),
	)
	if _, err := socket.Chat(ctx, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a22", "Answer briefly instead."); err != nil {
		t.Fatal(err)
	}
	conversationEqual(
		t,
		conversationLastText(t, conversationTranscript(ctx, t, w.server, &w.master, realAccountConversation).Blocks),
		"A short answer.",
	)
	inferences := w.inferences(t)
	conversationEqual(t, len(inferences), 2)
	if realAccountProcess(t, inferences[0]) == realAccountProcess(t, inferences[1]) {
		t.Fatal("stop kept CLI process")
	}
	realAccountReplayed(t, inferences[1], "Write a long answer.", "The long answer begins", "Answer briefly instead.")
	conversationRequest(
		ctx,
		t,
		w.server,
		&w.master,
		"PATCH",
		"/api/conversations/"+realAccountConversation,
		`{"model":{"providerId":"`+w.provider+`","modelId":"`+realAccountOtherModel+`"},"thinkingEffort":"medium"}`,
		200,
	)
	w.vendor.RespondAt(
		"/v1/messages",
		conversationMessage(
			conversationTextBlock(t, 0, "Another model answers."),
			"end_turn",
			`{"input_tokens":14,"output_tokens":1}`,
			3,
		),
	)
	if _, err := socket.Chat(ctx, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a23", "Once more."); err != nil {
		t.Fatal(err)
	}
	conversationEqual(
		t,
		conversationLastText(t, conversationTranscript(ctx, t, w.server, &w.master, realAccountConversation).Blocks),
		"Another model answers.",
	)
	inferences = w.inferences(t)
	conversationEqual(t, len(inferences), 3)
	if realAccountProcess(t, inferences[1]) == realAccountProcess(t, inferences[2]) {
		t.Fatal("model change kept CLI process")
	}
	body, ok := inferences[2].JSON(t).(map[string]any)
	if !ok {
		t.Fatal("request object")
	}
	conversationEqual(t, body["model"], any(realAccountOtherModel))
	config, ok := body["output_config"].(map[string]any)
	if !ok {
		t.Fatal("output_config missing")
	}
	conversationEqual(t, config["effort"], any("medium"))
	realAccountReplayed(
		t,
		inferences[2],
		"Write a long answer.",
		"The long answer begins",
		"Answer briefly instead.",
		"A short answer.",
		"Once more.",
	)
}

// TestAVendorErrorFailsTheRequestInTheCLIsWordsAndTheKeptProcessAnswersTheNext
// checks CLI error reporting and subsequent requests.
// Several seconds: the real CLI is installed and handles two scripted vendor requests.
func TestAVendorErrorFailsTheRequestInTheCLIsWordsAndTheKeptProcessAnswersTheNext(t *testing.T) {
	t.Parallel()
	ctx := t.Context()
	w := realAccountStart(ctx, t)
	socket := w.conversation(ctx, t)
	refusal := accountVendorResponse(
		`{"type":"error","error":{"type":"invalid_request_error",` +
			`"message":"The scripted vendor refuses this request."}}`,
	)
	refusal.Status = 400
	refusal.Headers = http.Header{"Content-Type": []string{"application/json"}}
	w.vendor.RespondAt("/v1/messages", refusal)
	if _, err := socket.Chat(ctx, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a31", "Hello?"); err != nil {
		t.Fatal(err)
	}
	blocks := conversationTranscript(ctx, t, w.server, &w.master, realAccountConversation).Blocks
	conversationEqual(t, conversationKinds(t, blocks), []string{"user", "error"})
	failed, ok := blocks[1].(*core.ErrorBlock)
	if !ok {
		t.Fatal(blocks[1])
	}
	if !strings.Contains(failed.Message, "The scripted vendor refuses this request.") {
		t.Fatal(failed.Message)
	}
	conversationEqual(t, len(w.usage(ctx, t)), 0)
	w.vendor.RespondAt(
		"/v1/messages",
		conversationMessage(
			conversationTextBlock(t, 0, "Better now."),
			"end_turn",
			`{"input_tokens":8,"output_tokens":1}`,
			2,
		),
	)
	if _, err := socket.Chat(ctx, "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a32", "Try again."); err != nil {
		t.Fatal(err)
	}
	blocks = conversationTranscript(ctx, t, w.server, &w.master, realAccountConversation).Blocks
	conversationEqual(t, conversationKinds(t, blocks[2:]), []string{"user", "text", "response"})
	conversationEqual(t, conversationLastText(t, blocks), "Better now.")
	inferences := w.inferences(t)
	conversationEqual(t, len(inferences), 2)
	conversationEqual(t, realAccountProcess(t, inferences[1]), realAccountProcess(t, inferences[0]))
	conversationEqual(t, w.usage(ctx, t)[0].Requests, uint64(1))
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

// conversationThinkingBlock streams reasoning and its replay signature.
func conversationThinkingBlock(t *testing.T, index int, text, signature string) string {
	return anthropicEvent(
		"content_block_start",
		fmt.Sprintf(
			`{"type":"content_block_start","index":%d,"content_block":{"type":"thinking",`+
				`"thinking":"","signature":""}}`,
			index,
		),
	) + anthropicEvent(
		"content_block_delta",
		fmt.Sprintf(
			`{"type":"content_block_delta","index":%d,"delta":{"type":"thinking_delta","thinking":%s}}`,
			index,
			conversationJSON(t, text),
		),
	) + anthropicEvent(
		"content_block_delta",
		fmt.Sprintf(
			`{"type":"content_block_delta","index":%d,"delta":{"type":"signature_delta","signature":%s}}`,
			index,
			conversationJSON(t, signature),
		),
	) + anthropicEvent(
		"content_block_stop",
		fmt.Sprintf(`{"type":"content_block_stop","index":%d}`, index),
	)
}

// conversationMessage completes one scripted response, with the reported usage.
func conversationMessage(blocks, stop, usage string, output uint64) providertest.MockResponse {
	return providertest.EventStream(
		conversationMessageStart(
			usage,
		) + blocks + anthropicEvent(
			"message_delta",
			fmt.Sprintf(
				`{"type":"message_delta","delta":{"stop_reason":"%s"},"usage":{"output_tokens":%d}}`,
				stop,
				output,
			),
		) + anthropicEvent(
			"message_stop",
			`{"type":"message_stop"}`,
		),
	)
}
