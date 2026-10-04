package claudecode_test

import (
	"context"
	"encoding/json"
	"errors"
	"slices"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

func TestNewProcessCLIContract(t *testing.T) {
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			equal(t, decoded(t, v["type"]), "user")
			equal(t, len(v), 2)
			checkJSON(t, v["message"], `{"role":"user","content":[{"type":"text","text":"hi"}]}`)
			c.say(`{"type":"system","subtype":"init","tools":[]}`)
			c.text("hel")
			c.text("lo")
			c.say(
				`{"type":"result","usage":{"input_tokens":10,"output_tokens":2,` +
					`"cache_read_input_tokens":7,"cache_creation_input_tokens":3}}`,
			)
		}
	})
	req := request(user("hi"))
	req.Thinking = &types.EffortConfig{Effort: "high"}
	events := collect(t.Context(), r, req)
	equal(
		t,
		events,
		[]provider.Event{
			textEvent("hel"),
			textEvent("lo"),
			&provider.Response{
				Usage: types.TokenUsage{InputTokens: 10, OutputTokens: 2, CacheReadTokens: 7, CacheWriteTokens: 3},
			},
		},
	)
	c := p.starts[0]
	equal(t, c.spawn.Command, "/demi/claude")
	equal(t, *c.spawn.CWD, "/demi/run")
	equal(t, c.spawn.Retained, true)
	equal(
		t,
		c.spawn.Args,
		[]string{
			"--print",
			"--output-format",
			"stream-json",
			"--verbose",
			"--input-format",
			"stream-json",
			"--include-partial-messages",
			"--no-session-persistence",
			"--safe-mode",
			"--disable-slash-commands",
			"--tools",
			"",
			"--permission-mode",
			"bypassPermissions",
			"--allow-dangerously-skip-permissions",
			"--model",
			"claude-test",
			"--system-prompt",
			"system",
			"--effort",
			"high",
		},
	)
	equal(t, c.spawn.Env.Mode, host.Overlay)
	want := map[string]*string{"CLAUDECODE": nil}
	for k, v := range map[string]string{
		"CLAUDE_CODE_OAUTH_TOKEN": testToken,
		"CLAUDE_CONFIG_DIR":       "/demi/config",
		"DISABLE_AUTOUPDATER":     "1",
		"DISABLE_AUTO_COMPACT":    "1",
		"MAX_MCP_OUTPUT_TOKENS":   "1000000",
	} {
		want[k] = &v
	}
	equal(t, c.spawn.Env.Values, want)
	equal(t, len(c.signals), 0)
	equal(t, c.closed, false)
	equal(t, c.finished, false)
}

func TestNoStartWithoutTokenOrPlacement(t *testing.T) {
	p, pool := testProvider(t, "http://127.0.0.1:9/catalog", "http://127.0.0.1:9/usage")
	account, _, err := pool.Active(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if err := pool.Remove(t.Context(), account); err != nil {
		t.Fatal(err)
	}
	placement := &scriptedPlacement{t: t}
	r := p.ProcessRuntime(placement)
	f := failure(t, collect(t.Context(), r, withTools(request(user("hi")))))
	equal(t, *f.Code, provider.AuthMissing)
	equal(t, f.Message, "No Claude Code account is signed in")
	equal(t, len(placement.starts), 0)
	p, _ = testProvider(t, "http://127.0.0.1:9/catalog", "http://127.0.0.1:9/usage")
	placement.fail = errors.New("Claude Code 2.1.3 could not be installed: the disk is full")
	f = failure(t, collect(t.Context(), p.ProcessRuntime(placement), request(user("hi"))))
	equal(t, f.Message, placement.fail.Error())
	equal(t, f.Code, (*provider.ErrorCode)(nil))
}

func TestLineBeforeInitializeAnswer(t *testing.T) {
	r, _ := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			if string(v["type"]) == `"control_request"` {
				c.say(`{"type":"system","subtype":"commands_changed","commands":[]}`)
				// A known line is kept too, so replay is observable.
				c.say(`{"type":"assistant","message":{"content":[{"type":"text","text":"before"}]}}`)
				c.initialized(v)
				return
			}
			equal(t, decoded(t, v["type"]), "user")
			equal(t, len(v), 2)
			checkJSON(t, v["message"], `{"role":"user","content":[{"type":"text","text":"hi"}]}`)
			c.text("hello")
			c.result(1, 1)
		}
	})
	equal(
		t,
		collect(t.Context(), r, withTools(request(user("hi")))),
		[]provider.Event{textEvent("before"), textEvent("hello"), response(1, 1)},
	)
}

func TestKeptProcessContinuationAndRestarts(t *testing.T) {
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			if c.initialized(v) {
				return
			}
			equal(t, decoded(t, v["type"]), "user")
			equal(t, len(v), 2)
			checkJSON(t, v["message"], `{"role":"user","content":[{"type":"text","text":"do work"}]}`)
			c.text("one")
			c.say(
				`{"type":"result","usage":{"input_tokens":30,"output_tokens":300,` +
					`"iterations":[{"input_tokens":10,"output_tokens":100},` +
					`{"input_tokens":20,"output_tokens":200,"iterations":"not read on ` +
					`an individual call"}]}}`,
			)
		}
	})
	first := withTools(request(user("do work")))
	equal(t, collect(t.Context(), r, first), []provider.Event{textEvent("one"), response(20, 200)})
	p.setup = func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			if !c.initialized(v) {
				c.result(1, 1)
			}
		}
	}
	c := p.starts[0]
	c.onWrite = func(v map[string]json.RawMessage) {
		equal(t, decoded(t, v["type"]), "user")
		equal(t, len(v), 2)
		checkJSON(t, v["message"], `{"role":"user","content":[{"type":"text","text":"second question"}]}`)
		c.text("two")
		c.result(1, 1)
	}
	second := withTools(request(user("do work"), &provider.AssistantText{Text: "one"}, user("second question")))
	equal(t, collect(t.Context(), r, second), []provider.Event{textEvent("two"), response(1, 1)})
	equal(t, len(p.starts), 1)
	equal(t, len(c.input), 3)
	equal(t, len(c.signals), 0)
	second.Items[0] = user("do other work")
	equal(t, collect(t.Context(), r, second), []provider.Event{response(1, 1)})
	equal(t, c.end, host.ProcessEnd{Kind: host.ProcessSignalled, Signal: string(host.Terminate)})
	equal(t, c.signals, []host.Signal{host.Terminate})
	replay := p.starts[1]
	equal(t, len(replay.input), 2)
	checkJSON(
		t,
		replay.input[1],
		`{"type":"user","message":{"role":"user",`+
			`"content":[{"type":"text","text":"User: do other `+
			`work\n\nAssistant: one\n\nUser: second question"}]}}`,
	)
	second.ModelID = "claude-other"
	second.Tools = nil
	collect(t.Context(), r, second)
	equal(t, replay.signals, []host.Signal{host.Terminate})
	other := p.starts[2]
	modelIndex := slices.Index(other.spawn.Args, "--model")
	if modelIndex < 0 || modelIndex+1 >= len(other.spawn.Args) {
		t.Fatal("missing model argument")
	}
	equal(t, other.spawn.Args[modelIndex+1], "claude-other")
	second = withTools(second)
	collect(t.Context(), r, second)
	equal(t, other.signals, []host.Signal{host.Terminate})
	equal(t, len(p.starts), 4)
	// A changed effort is fixed at spawn, just like the model.
	second.Items = append(second.Items, user("again"))
	second.Thinking = &types.EffortConfig{Effort: "low"}
	collect(t.Context(), r, second)
	equal(t, len(p.starts), 5)
}

func TestTranscriptSpeakersAndMedia(t *testing.T) {
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			if !c.initialized(v) {
				c.result(3, 1)
			}
		}
	})
	signature := "signed"
	req := withTools(
		request(
			&provider.UserMessage{
				Content: []provider.UserPart{
					&provider.TextPart{Text: "previous work"},
					&provider.ImagePart{Medium: &provider.MediaBytes{Data: []byte("png"), MediaType: "image/png"}},
				},
			},
			&provider.AssistantThinking{ModelID: "claude-test", Text: "thinking", Signature: &signature},
			toolUse("tool-1", "pwd"),
			toolOutput("tool-1", "/tmp"),
			user("continue"),
		),
	)
	equal(t, collect(t.Context(), r, req), []provider.Event{response(3, 1)})
	equal(t, len(p.starts[0].input), 2)
	checkJSON(
		t,
		p.starts[0].input[1],
		`{"type":"user","message":{"role":"user",`+
			`"content":[{"type":"text","text":"User: previous work"},`+
			`{"type":"image","source":{"type":"base64","media_type":"image/`+
			`png","data":"cG5n"}},{"type":"text","text":"Assistant: [Earlier `+
			`in this conversation I called the tool shell_exec with input: `+
			`{\"script\":\"pwd\"}.\n\nIt returned from shell_exec: /tmp]`+
			`\n\nUser: continue"}]}}`,
	)
}

func TestProcessExitStatusAndStderr(t *testing.T) {
	r, p := fixture(t, nil)
	for _, tc := range []struct {
		code         int32
		stderr, want string
	}{
		{0, "", ""},
		{1, "the configuration home cannot be written\n", "the configuration home cannot be written"},
		{2, "", "Claude Code exited with code 2"},
		{1, strings.Repeat("x", 100*1024), strings.Repeat("x", 64*1024-3) + "end"},
	} {
		p.setup = func(c *scriptedCLI) {
			c.onWrite = func(map[string]json.RawMessage) {
				if tc.stderr != "" {
					c.output <- host.ProcessOutput{Stream: types.StreamKind("stderr"), Bytes: []byte(tc.stderr)}
				}
				if len(tc.stderr) == 100*1024 {
					c.output <- host.ProcessOutput{Stream: types.StreamKind("stderr"), Bytes: []byte("end")}
				}
				c.finish(host.ProcessEnd{Kind: host.ProcessExited, ExitCode: tc.code})
			}
		}
		events := collect(t.Context(), r, request(user("hi")))
		if tc.want == "" {
			equal(t, len(events), 0)
		} else {
			equal(t, failure(t, events).Message, tc.want)
		}
	}
	equal(t, len(p.starts), 4)
}

func TestKeptProcessThatExitedIsReplaced(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r, p := fixture(t, func(c *scriptedCLI) {
			c.onWrite = func(map[string]json.RawMessage) {
				c.result(2, 2)
			}
		})
		collect(t.Context(), r, request(user("hi")))
		p.starts[0].finish(host.ProcessEnd{Kind: host.ProcessLost, Reason: "runner disconnected"})
		synctest.Wait()
		equal(t, collect(t.Context(), r, request(user("hi"), user("again"))), []provider.Event{response(2, 2)})
		equal(t, len(p.starts), 2)
		checkJSON(
			t,
			p.starts[1].input[0],
			`{"type":"user","message":{"role":"user",`+
				`"content":[{"type":"text","text":"hi"},{"type":"text","text":"again"}]}}`,
		)
	})
}

func TestCancelledRunClosesWithoutEvent(t *testing.T) {
	for _, ignore := range []bool{false, true} {
		t.Run(map[bool]string{false: "terminate", true: "kill"}[ignore], func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				ctx, cancel := context.WithCancel(t.Context())
				defer cancel()
				r, p := fixture(t, func(c *scriptedCLI) {
					c.ignoreTerminate = ignore
					c.onWrite = func(map[string]json.RawMessage) {
						if ignore {
							cancel()
						} else {
							c.text("partial")
						}
					}
				})
				start := time.Now()
				var events []provider.Event
				for event := range r.Run(ctx, request(user("hi"))) {
					events = append(events, event)
					cancel()
				}
				if ignore {
					equal(t, len(events), 0)
				} else {
					equal(t, events, []provider.Event{textEvent("partial")})
				}
				want := []host.Signal{host.Terminate}
				elapsed := time.Duration(0)
				if ignore {
					want = append(want, host.Kill)
					elapsed = 5 * time.Second
				}
				equal(t, p.starts[0].signals, want)
				equal(t, p.starts[0].closedWhileRunning, false)
				signal := host.Terminate
				if ignore {
					signal = host.Kill
				}
				equal(t, p.starts[0].end, host.ProcessEnd{Kind: host.ProcessSignalled, Signal: string(signal)})
				equal(t, time.Since(start), elapsed)
			})
		})
	}
}

func TestEarlyIteratorStopAndRuntimeClose(t *testing.T) {
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(map[string]json.RawMessage) {
			c.text("first")
		}
	})
	for event := range r.Run(t.Context(), request(user("hi"))) {
		equal(t, event, textEvent("first"))
		break
	}
	equal(t, p.starts[0].closed, true)
	equal(t, p.starts[0].closedWhileRunning, true)
	equal(t, len(p.starts[0].input), 1)
	equal(t, len(p.starts[0].signals), 0)
	p.setup = func(c *scriptedCLI) {
		c.onWrite = func(map[string]json.RawMessage) {
			c.result(1, 1)
		}
	}
	collect(t.Context(), r, request(user("hi")))
	fresh := r.Fresh()
	collect(t.Context(), fresh, request(user("elsewhere")))
	equal(t, p.starts[1].closed, false)
	equal(t, len(p.starts[1].signals), 0)
	equal(t, len(p.starts[2].input), 1)
	checkJSON(
		t,
		p.starts[2].input[0],
		`{"type":"user","message":{"role":"user","content":[{"type":"text","text":"elsewhere"}]}}`,
	)
	if err := fresh.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	equal(t, p.starts[2].closed, true)
	equal(t, p.starts[1].closed, false)
	if err := r.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	equal(t, p.starts[1].closed, true)
}

func TestResultErrorKeepsProcessButBrokenLineCloses(t *testing.T) {
	result := `{"type":"result","is_error":true,"result":"context window ` +
		`exceeded","errors":["input is too long"],` +
		`"usage":{"input_tokens":200000,"output_tokens":0}}`
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(map[string]json.RawMessage) {
			c.say(result)
		}
	})
	var events []provider.Event
	for event := range r.Run(t.Context(), request(user("huge"))) {
		events = append(events, event)
		break
	}
	f := failure(t, events)
	equal(t, f.Message, "context window exceeded\ninput is too long")
	equal(t, *f.Code, provider.ContextLengthExceeded)
	equal(t, *f.Diagnostics.Upstream, result)
	c := p.starts[0]
	equal(t, c.closed, false)
	equal(t, len(c.signals), 0)
	c.onWrite = func(v map[string]json.RawMessage) {
		equal(t, decoded(t, v["type"]), "user")
		equal(t, len(v), 2)
		checkJSON(t, v["message"], `{"role":"user","content":[{"type":"text","text":"smaller"}]}`)
		c.say(`{"type":"result","usage":{"input_tokens":"many"}}`)
	}
	f = failure(t, collect(t.Context(), r, request(user("huge"), user("smaller"))))
	if !strings.HasPrefix(f.Message, "Claude Code sent a line Demi cannot read") {
		t.Fatal(f.Message)
	}
	equal(t, f.Code, (*provider.ErrorCode)(nil))
	equal(t, c.signals, []host.Signal{host.Terminate})
	equal(t, len(p.starts), 1)
}

func TestRetryWithNoNewInputReplays(t *testing.T) {
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(map[string]json.RawMessage) {
			c.say(`{"type":"result","is_error":true,"result":"overloaded"}`)
		}
	})
	for event := range r.Run(t.Context(), request(user("hi"))) {
		if _, ok := event.(*provider.Error); !ok {
			t.Fatalf("expected error: %#v", event)
		}
		break
	}
	p.setup = func(c *scriptedCLI) {
		c.onWrite = func(map[string]json.RawMessage) {
			c.result(1, 1)
		}
	}
	equal(t, collect(t.Context(), r, request(user("hi"))), []provider.Event{response(1, 1)})
	equal(t, len(p.starts), 2)
	equal(t, p.starts[0].signals, []host.Signal{host.Terminate})
	checkJSON(
		t,
		p.starts[1].input[0],
		`{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}`,
	)
}
