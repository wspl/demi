package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"
)

func TestDevSeededAccountEchoAndShutdown(t *testing.T) {
	t.Skip("b-backend API checkpoint: cmd/demi-backend and internal/backend/backendtest/testdata/scripted-machines still panic; nativeConfig is private. Full programtest scenario waits for those implementations and the API request in the report.")
}
func TestEchoStreamsLastUserTextAndCloses(t *testing.T) {
	echo, err := startEcho(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := echo.close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	for _, scenario := range []struct {
		body, want string
		status     int
	}{
		{`{"messages":[{"role":"user","content":"hello"}]}`, "Echo: hello", 200},
		{`{"messages":[{"role":"user","content":"old"},{"role":"assistant","content":"answer"},{"role":"user","content":[{"type":"image"},{"type":"text","text":"one"},{"type":"text","text":"two"}]}]}`, "Echo: one\ntwo", 200},
		{`{"messages":[]}`, "Echo: ", 200},
		{`{"messages":[{"role":"user","content":[{"type":"text"}]}]}`, "", 422},
		{`{"messages":[{"role":"other","content":"bad"}]}`, "", 422},
	} {
		request, err := http.NewRequestWithContext(t.Context(), http.MethodPost, echo.url+"/messages", strings.NewReader(scenario.body))
		if err != nil {
			t.Fatal(err)
		}
		response, err := http.DefaultClient.Do(request)
		if err != nil {
			t.Fatal(err)
		}
		data, err := io.ReadAll(response.Body)
		closeErr := response.Body.Close()
		if err != nil || closeErr != nil {
			t.Fatal(err, closeErr)
		}
		if response.StatusCode != scenario.status {
			t.Fatalf("status = %s, body = %s", response.Status, data)
		}
		if scenario.status != 200 {
			continue
		}
		var text strings.Builder
		var events []string
		for line := range strings.SplitSeq(string(data), "\n") {
			if name, ok := strings.CutPrefix(line, "event: "); ok {
				events = append(events, name)
			}
			if payload, ok := strings.CutPrefix(line, "data: "); ok {
				var event struct {
					Type  string `json:"type"`
					Delta struct {
						Text string `json:"text"`
					} `json:"delta"`
				}
				if err := json.Unmarshal([]byte(payload), &event); err != nil {
					t.Fatal(err)
				}
				if event.Type == "content_block_delta" {
					text.WriteString(event.Delta.Text)
				}
			}
		}
		if text.String() != scenario.want || len(events) < 6 || events[0] != "message_start" || events[len(events)-1] != "message_stop" {
			t.Fatalf("stream = %q, events = %v", text.String(), events)
		}
	}
}
