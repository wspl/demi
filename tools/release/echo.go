//go:build unix

package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

type echoServer struct {
	url    string
	server *http.Server
	done   chan struct{}
	err    error
}

func startEcho(ctx context.Context) (*echoServer, error) {
	var config net.ListenConfig
	listener, err := config.Listen(ctx, "tcp", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}
	mux := http.NewServeMux()
	mux.HandleFunc("POST /v1/messages", echoMessages)
	echo := &echoServer{
		url:    "http://" + listener.Addr().String() + "/v1",
		server: &http.Server{Handler: mux},
		done:   make(chan struct{}),
	}
	go func() {
		echo.err = echo.server.Serve(listener)
		close(echo.done)
	}()
	return echo, nil
}

func (e *echoServer) close(_ context.Context) error {
	err := e.server.Close()
	<-e.done
	if !errors.Is(e.err, http.ErrServerClosed) {
		err = errors.Join(err, e.err)
	}
	return err
}

func echoText(data []byte) (string, error) {
	if err := contract.CheckJSON(data); err != nil {
		return "", err
	}
	var request struct {
		Messages *[]json.RawMessage `json:"messages"`
	}
	if err := json.Unmarshal(data, &request); err != nil {
		return "", err
	}
	if request.Messages == nil {
		return "", errors.New("messages is required")
	}
	last := ""
	for _, raw := range *request.Messages {
		var message struct {
			Role    *string         `json:"role"`
			Content json.RawMessage `json:"content"`
		}
		if err := json.Unmarshal(raw, &message); err != nil {
			return "", err
		}
		if message.Role == nil || (*message.Role != "user" && *message.Role != "assistant") {
			return "", errors.New("invalid message role")
		}
		text, err := echoContent(message.Content)
		if err != nil {
			return "", err
		}
		if *message.Role == "user" {
			last = text
		}
	}
	return last, nil
}

// Vendor stream structures are local protocol output, not Demi contracts.
type echoUsage struct {
	InputTokens  int `json:"input_tokens,omitempty"`
	OutputTokens int `json:"output_tokens"`
}
type echoMessage struct {
	ID      string    `json:"id"`
	Type    string    `json:"type"`
	Role    string    `json:"role"`
	Model   string    `json:"model"`
	Content []string  `json:"content"`
	Usage   echoUsage `json:"usage"`
}
type echoTextBlock struct {
	Type string `json:"type"`
	Text string `json:"text"`
}
type echoStopDelta struct {
	StopReason   string  `json:"stop_reason"`
	StopSequence *string `json:"stop_sequence"`
}

func echoMessages(w http.ResponseWriter, r *http.Request) {
	data, err := io.ReadAll(r.Body)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	text, err := echoText(data)
	if err != nil {
		http.Error(w, err.Error(), http.StatusUnprocessableEntity)
		return
	}
	w.Header().Set("Content-Type", "text/event-stream")
	send := func(name string, value any) bool {
		return sendEchoEvent(w, name, value)
	}
	if !send("message_start", struct {
		Type    string      `json:"type"`
		Message echoMessage `json:"message"`
	}{"message_start", echoMessage{"msg_echo", "message", "assistant", "echo", []string{}, echoUsage{1, 1}}}) {
		return
	}
	if !send("content_block_start", struct {
		Type  string        `json:"type"`
		Index int           `json:"index"`
		Block echoTextBlock `json:"content_block"`
	}{"content_block_start", 0, echoTextBlock{"text", ""}}) {
		return
	}
	words := strings.SplitAfter("Echo: "+text, " ")
	if words[len(words)-1] == "" {
		words = words[:len(words)-1]
	}
	for _, word := range words {
		if !send("content_block_delta", struct {
			Type  string        `json:"type"`
			Index int           `json:"index"`
			Delta echoTextBlock `json:"delta"`
		}{"content_block_delta", 0, echoTextBlock{"text_delta", word}}) {
			return
		}
	}
	if !send("content_block_stop", struct {
		Type  string `json:"type"`
		Index int    `json:"index"`
	}{"content_block_stop", 0}) {
		return
	}
	if !send("message_delta", struct {
		Type  string        `json:"type"`
		Delta echoStopDelta `json:"delta"`
		Usage echoUsage     `json:"usage"`
	}{"message_delta", echoStopDelta{StopReason: "end_turn"}, echoUsage{OutputTokens: len(words)}}) {
		return
	}
	send("message_stop", struct {
		Type string `json:"type"`
	}{"message_stop"})
}

func echoContent(content json.RawMessage) (string, error) {
	var text string
	if err := json.Unmarshal(content, &text); err != nil || string(content) == "null" {
		var err error
		text, err = echoBlocks(content)
		if err != nil {
			return "", err
		}
	}
	return text, nil
}

func echoBlocks(content json.RawMessage) (string, error) {
	var blocks *[]json.RawMessage
	if err := json.Unmarshal(content, &blocks); err != nil {
		return "", err
	}
	if blocks == nil {
		return "", errors.New("message content is required")
	}
	var texts []string
	for _, raw := range *blocks {
		text, err := echoBlock(raw)
		if err != nil {
			return "", err
		}
		if text != nil {
			texts = append(texts, *text)
		}
	}
	return strings.Join(texts, "\n"), nil
}

// echoBlock returns nil for non-text blocks so they contribute no text to the echo.
func echoBlock(raw json.RawMessage) (*string, error) {
	var tag struct {
		Type *string `json:"type"`
	}
	if err := json.Unmarshal(raw, &tag); err != nil {
		return nil, err
	}
	if tag.Type == nil {
		return nil, errors.New("block type is required")
	}
	if *tag.Type == "text" {
		var block struct {
			Text *string `json:"text"`
		}
		if err := json.Unmarshal(raw, &block); err != nil {
			return nil, err
		}
		if block.Text == nil {
			return nil, errors.New("text block requires text")
		}
		return block.Text, nil
	}
	return nil, nil
}

func sendEchoEvent(w http.ResponseWriter, name string, value any) bool {
	data, err := contract.EncodeJSON(value)
	if err != nil {
		return false
	}
	if _, err := fmt.Fprintf(w, "event: %s\ndata: %s\n\n", name, data); err != nil {
		return false
	}
	if f, ok := w.(http.Flusher); ok {
		f.Flush()
	}
	return true
}
