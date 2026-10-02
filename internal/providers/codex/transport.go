package codex

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"iter"
	"net/http"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/provider"
)

func (p *Provider) open(ctx context.Context, client *http.Client, headers http.Header, body []byte) (events, func(), error) {
	if p.config.Transport == SSE {
		return p.sse(ctx, client, headers, body)
	}
	now := p.clock.Now()
	p.mu.Lock()
	bypass := p.unreachableUntil != "" && now < p.unreachableUntil
	p.mu.Unlock()
	if p.config.Transport == Auto && bypass {
		return p.sse(ctx, client, headers, body)
	}
	stream, cleanup, err := p.websocket(ctx, client, headers, body)
	if p.config.Transport == WebSocket {
		return stream, cleanup, err
	}
	if err == nil {
		next, stop := iter.Pull2(stream)
		first, firstErr, ok := next()
		if ok && firstErr == nil {
			return func(yield func(provider.Received, error) bool) {
					if !yield(first, nil) {
						return
					}
					for {
						value, err, ok := next()
						if !ok || !yield(value, err) {
							return
						}
					}
				}, func() {
					stop()
					cleanup()
				}, nil
		}
		stop()
		cleanup()
	} else {
		var rejection *refusal
		if !errors.As(err, &rejection) {
			ms, _ := p.clock.Now().Millisecond()
			until := provider.UnixSeconds(float64(ms)/1000 + 600)
			if until != nil {
				p.mu.Lock()
				p.unreachableUntil = *until
				p.mu.Unlock()
			}
		}
	}
	if ctx.Err() != nil {
		return nil, nil, ctx.Err()
	}
	return p.sse(ctx, client, headers, body)
}

func (p *Provider) sse(ctx context.Context, client *http.Client, headers http.Header, body []byte) (events, func(), error) {
	requestCtx, cancel := context.WithCancel(ctx)
	request, err := http.NewRequestWithContext(requestCtx, http.MethodPost, p.responsesURL, bytes.NewReader(body))
	if err != nil {
		cancel()
		return nil, nil, err
	}
	request.Header = headers.Clone()
	// Only header arrival is bounded. Join the timer callback before handing the body off.
	fired := make(chan struct{})
	timer := time.AfterFunc(p.config.HeaderTimeout, func() {
		cancel()
		close(fired)
	})
	response, err := client.Do(request)
	timedOut := !timer.Stop()
	if timedOut {
		<-fired
		cancel()
		if response != nil {
			_ = response.Body.Close()
		} // The header deadline already ended the request.
		f := provider.NoAnswer(fmt.Sprintf("Codex SSE response headers timed out after %dms", p.config.HeaderTimeout.Milliseconds()))
		return nil, nil, &f
	}
	if err != nil {
		cancel()
		f := provider.TransportFailure("Codex", withoutURL(err))
		return nil, nil, &f
	}
	p.quota.Observe(&provider.HTTPObservation{Status: response.StatusCode, Headers: response.Header})
	cleanup := func() {
		cancel()
		_ = response.Body.Close()
	} // Stream errors are read by the mapper; close releases the body.
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		data, err := io.ReadAll(response.Body)
		if err != nil {
			data = nil
		} // Status and headers remain usable when a refusal's body cannot be read.
		cleanup()
		return nil, nil, &refusal{status: response.StatusCode, headers: response.Header, body: strings.ToValidUTF8(string(data), "�")}
	}
	return provider.ResponsesSSEEvents(ctx, response.Body, "Codex"), cleanup, nil
}

func (p *Provider) websocket(ctx context.Context, client *http.Client, headers http.Header, body []byte) (events, func(), error) {
	headers = headers.Clone()
	headers.Del("Accept")
	headers.Del("Content-Type")
	headers.Set("Openai-Beta", "responses_websockets=2026-02-06")
	transport := &handshakeTransport{base: client.Transport}
	if transport.base == nil {
		transport.base = http.DefaultTransport
	}
	dialClient := *client
	dialClient.Transport = transport
	// tungstenite treats redirects as handshake refusals.
	dialClient.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	activity := make(chan struct{}, 1)
	pulse := func() {
		select {
		case activity <- struct{}{}:
		default:
		}
	}
	connectCtx, cancel := context.WithTimeout(ctx, p.config.ConnectTimeout)
	socket, response, err := websocket.Dial(connectCtx, p.responsesURL, &websocket.DialOptions{
		HTTPClient: &dialClient, HTTPHeader: headers,
		OnPingReceived: func(context.Context, []byte) bool {
			pulse()
			return true
		},
		OnPongReceived: func(context.Context, []byte) { pulse() },
	})
	expired := errors.Is(connectCtx.Err(), context.DeadlineExceeded)
	cancel()
	if response != nil {
		p.quota.Observe(&provider.HTTPObservation{Status: response.StatusCode, Headers: response.Header})
	}
	if err != nil {
		if transport.refusal != nil && !expired {
			if response != nil && response.Body != nil {
				_ = response.Body.Close()
			} // The complete refusal is already retained.
			return nil, nil, transport.refusal
		}
		message := fmt.Sprintf("Codex WebSocket connect failed: %v", withoutURL(err))
		if expired {
			message = fmt.Sprintf("Codex WebSocket connect timed out after %dms", p.config.ConnectTimeout.Milliseconds())
		}
		f := provider.NoAnswer(message)
		return nil, nil, &f
	}
	// Match tungstenite's message limit. coder/websocket has no independent
	// frame-size option (Rust also limits a frame to 16 MiB); see the handoff request.
	socket.SetReadLimit(64 * 1024 * 1024)
	cleanup := func() { _ = socket.CloseNow() } // Release even when the peer has already closed.
	message := append([]byte(`{"type":"response.create",`), body[1:]...)
	if err := socket.Write(ctx, websocket.MessageText, message); err != nil {
		f := provider.NoAnswer(fmt.Sprintf("Codex WebSocket send failed: %v", err))
		return func(yield func(provider.Received, error) bool) { yield(provider.Received{}, &f) }, cleanup, nil
	}
	return p.websocketEvents(ctx, socket, activity), cleanup, nil
}

type socketMessage struct {
	kind websocket.MessageType
	data []byte
	err  error
}

// coder/websocket exposes only Close (send and wait for the peer, with its
// built-in five-second deadlines) and CloseNow (send nothing). Normal close
// reasons require Close; the missing send-only operation is a handoff request.
func (p *Provider) websocketEvents(ctx context.Context, socket *websocket.Conn, activity <-chan struct{}) events {
	return func(yield func(provider.Received, error) bool) {
		for {
			// Read without the request context so cancellation can send the normal aborted close.
			// The run owns and joins this reader after closing the connection on every exit.
			received := make(chan struct{})
			var message socketMessage
			go func() {
				defer close(received)
				message.kind, message.data, message.err = socket.Read(context.WithoutCancel(ctx))
			}()
			var timer *time.Timer
			var idle <-chan time.Time
			if p.config.StreamIdleTimeout != nil {
				timer = time.NewTimer(*p.config.StreamIdleTimeout)
				idle = timer.C
			}
		waiting:
			for {
				select {
				case <-activity:
					if timer != nil {
						timer.Reset(*p.config.StreamIdleTimeout)
					}
				case <-ctx.Done():
					_ = socket.Close(websocket.StatusNormalClosure, "aborted") // Best effort; the stream ends even if the peer already closed.
					<-received
					if timer != nil {
						timer.Stop()
					}
					return
				case <-idle:
					_ = socket.Close(websocket.StatusNormalClosure, "idle_timeout") // Best effort; the stream ends even if the peer already closed.
					<-received
					f := provider.NoAnswer(fmt.Sprintf("Codex WebSocket stream idled for %dms", p.config.StreamIdleTimeout.Milliseconds()))
					yield(provider.Received{}, &f)
					return
				case <-received:
					if timer != nil {
						timer.Stop()
					}
					break waiting
				}
			}
			if ctx.Err() != nil {
				_ = socket.Close(websocket.StatusNormalClosure, "aborted") // Best effort; the stream ends even if the peer already closed.
				return
			}
			if message.err != nil {
				if websocket.CloseStatus(message.err) >= 0 {
					return
				}
				f := provider.NoAnswer(fmt.Sprintf("Codex WebSocket failed: %v", message.err))
				yield(provider.Received{}, &f)
				return
			}
			text := string(message.data)
			if !utf8.Valid(message.data) {
				if message.kind == websocket.MessageText {
					f := provider.NoAnswer("Codex WebSocket failed: UTF-8 encoding error")
					yield(provider.Received{}, &f)
					return
				}
				f := provider.ProtocolFailure("Codex API stream sent a binary message that is not UTF-8 text", strings.ToValidUTF8(text, "�"))
				yield(provider.Received{}, &f)
				return
			}
			event, err := decodeMessage(text)
			if err != nil {
				f := provider.Undecodable("Codex", err, text)
				yield(provider.Received{}, &f)
				return
			}
			if event == nil {
				continue
			}
			terminal := false
			switch event.Event.(type) {
			case *provider.ResponsesCompleted, *provider.ResponsesFailed, *provider.ResponsesIncomplete, *provider.ResponsesError:
				terminal = true
			case *provider.ResponsesItemAdded, *provider.ResponsesItemDone, *provider.ResponsesTextDelta, *provider.ResponsesReasoningTextDelta, *provider.ResponsesReasoningSummaryDelta, *provider.ResponsesArgumentsDelta, *provider.ResponsesArgumentsDone:
			}
			if terminal {
				_ = socket.Close(websocket.StatusNormalClosure, "response_done") // Best effort; the stream ends even if the peer already closed.
			}
			if !yield(*event, nil) || terminal {
				return
			}
		}
	}
}
func decodeMessage(text string) (*provider.Received, error) {
	envelope, err := provider.DecodeUntagged[struct {
		Type     *string          `json:"type"`
		Event    *json.RawMessage `json:"event"`
		Response json.RawMessage  `json:"response" wire:"optional"`
	}](text)
	if err != nil {
		return nil, err
	}
	if envelope.Event != nil {
		if _, err := provider.DecodeUntagged[map[string]json.RawMessage](string(*envelope.Event)); err != nil {
			return nil, err
		}
	}
	payload := text
	if envelope.Type != nil && *envelope.Type == "response.done" {
		response := envelope.Response
		if len(response) == 0 {
			response = json.RawMessage("null")
		}
		encoded, err := provider.JSONBody(struct {
			Type     string          `json:"type"`
			Response json.RawMessage `json:"response"`
		}{Type: "response.completed", Response: response})
		if err != nil {
			return nil, err
		}
		payload = string(encoded)
	} else if envelope.Event != nil {
		payload, err = provider.ToolArguments(*envelope.Event)
		if err != nil {
			return nil, err
		}
	}
	event, err := provider.DecodeResponsesFrame(payload)
	if event != nil {
		event.Text = text
	}
	return event, err
}
