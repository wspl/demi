package codex

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"io"
	"iter"
	"net/http"
	"strings"
	"sync/atomic"
	"time"
	"unicode/utf8"

	"github.com/coder/websocket"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

func (p *Provider) Runtime(env provider.RuntimeEnv) (provider.ProviderRuntime, error) {
	return &runtimeProvider{shared: p, http: env.HTTP}, nil
}

type runtimeProvider struct {
	shared *Provider
	http   *http.Client
}

func (r *runtimeProvider) Fresh() provider.ProviderRuntime {
	return &runtimeProvider{shared: r.shared, http: r.http}
}
func (*runtimeProvider) Close(context.Context) error { return nil }
func (*runtimeProvider) RequestLimits(core.Model) provider.RequestLimits {
	return provider.OpenAIRequestLimits()
}
func (r *runtimeProvider) Run(ctx context.Context, request provider.InferenceRequest) iter.Seq[provider.ProviderEvent] {
	return func(yield func(provider.ProviderEvent) bool) {
		if ctx.Err() != nil {
			return
		}
		encoded, failure := provider.EncodeBody("Codex", func() []byte { return body(request) })
		if ctx.Err() != nil {
			return
		}
		if failure != nil {
			yield(provider.FailureEvent{Failure: *failure})
			return
		}
		var refused *provider.Secret
		for {
			secret, failure := r.shared.auth.credentials(ctx, r.http, refused)
			if ctx.Err() != nil {
				return
			}
			if failure != nil {
				yield(provider.FailureEvent{Failure: failure.Failure()})
				return
			}
			headers := r.shared.inferenceHeaders(secret, request)
			stream, refusal, failed := r.open(ctx, headers, encoded)
			if ctx.Err() != nil {
				if stream != nil {
					stream.close()
				}
				return
			}
			if refusal != nil && refusal.status == 401 && refused == nil {
				refused = &secret.AccessToken
				continue
			}
			if refusal != nil {
				yield(provider.FailureEvent{Failure: r.shared.refused(*refusal)})
				return
			}
			if failed != nil {
				yield(provider.FailureEvent{Failure: *failed})
				return
			}
			defer stream.close()
			mapper := provider.NewResponsesMapper(provider.Vendor{Label: "Codex", Reader: ReadFailure, Clock: r.shared.clock}, SignatureTag)
			for frame, failure := range stream.frames {
				if ctx.Err() != nil {
					return
				}
				if failure != nil {
					yield(provider.FailureEvent{Failure: *failure})
					return
				}
				events, ended := mapper.FrameWithRecord(frame.data, frame.record)
				if ended && stream.socket != nil {
					stream.socket.Close(websocket.StatusNormalClosure, "response_done")
				} // Best effort; a closed peer cannot receive the frame.
				for _, event := range events {
					if ctx.Err() != nil || !yield(event) {
						return
					}
				}
				if ended {
					return
				}
			}
			if ctx.Err() == nil {
				yield(provider.Response{})
			}
			return
		}
	}
}

type refusal struct {
	status  uint16
	headers http.Header
	body    string
}
type received struct{ data, record string }
type opened struct {
	frames iter.Seq2[received, *provider.ProviderFailure]
	close  func()
	socket *websocket.Conn
}

func (r *runtimeProvider) open(ctx context.Context, headers http.Header, body []byte) (*opened, *refusal, *provider.ProviderFailure) {
	p := r.shared
	if p.config.Transport == TransportSSE || (p.config.Transport != TransportWebSocket && p.unreachable.Load()) {
		return r.sse(ctx, headers, body)
	}
	stream, refusal, failure := r.websocket(ctx, headers, body)
	if p.config.Transport == TransportWebSocket {
		return stream, refusal, failure
	}
	if failure != nil {
		p.unreachable.Store(true)
	}
	if stream != nil {
		next, stop := iter.Pull2(stream.frames)
		frame, failure, ok := next()
		if ctx.Err() != nil {
			stop()
			stream.close()
			return nil, nil, nil
		}
		if ok && failure == nil {
			// Only a registered, valid event ends the fallback window.
			probe := provider.NewResponsesMapper(provider.Vendor{Label: "Codex", Reader: ReadFailure, Clock: p.clock}, SignatureTag)
			_, _, protocolError := probe.FrameChecked(frame.data, frame.record)
			if protocolError == nil {
				oldClose := stream.close
				stream.close = func() { stop(); oldClose() }
				stream.frames = func(yield func(received, *provider.ProviderFailure) bool) {
					if !yield(frame, nil) {
						return
					}
					for {
						frame, failure, ok := next()
						if !ok || !yield(frame, failure) {
							return
						}
					}
				}
				return stream, nil, nil
			}
		}
		stop()
		stream.close()
	}
	if ctx.Err() != nil {
		return nil, nil, nil
	}
	return r.sse(ctx, headers, body)
}
func (r *runtimeProvider) sse(ctx context.Context, headers http.Header, body []byte) (*opened, *refusal, *provider.ProviderFailure) {
	requestCtx, cancel := context.WithCancel(ctx)
	request, err := http.NewRequestWithContext(requestCtx, http.MethodPost, r.shared.responsesURL, bytes.NewReader(body))
	if err != nil {
		cancel()
		failure := provider.NoAnswer("Codex SSE request could not be built")
		return nil, nil, &failure
	}
	request.Header = headers.Clone()
	var expired atomic.Bool
	timer := time.AfterFunc(r.shared.config.HeaderTimeout, func() { expired.Store(true); cancel() })
	response, err := r.http.Do(request)
	timer.Stop()
	if err != nil {
		cancel()
		failure := provider.TransportFailure("Codex", err)
		if expired.Load() {
			failure = provider.NoAnswer(fmt.Sprintf("Codex SSE response headers timed out after %dms", r.shared.config.HeaderTimeout.Milliseconds()))
		}
		return nil, nil, &failure
	}
	r.shared.quota.Observe(provider.Observation{Status: uint16(response.StatusCode), Headers: response.Header})
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		raw, readErr := io.ReadAll(response.Body)
		response.Body.Close()
		cancel()
		if readErr != nil {
			raw = nil
		}
		return nil, &refusal{status: uint16(response.StatusCode), headers: response.Header.Clone(), body: strings.ToValidUTF8(string(raw), "\uFFFD")}, nil
	}
	stream := &opened{close: func() { response.Body.Close(); cancel() }}
	stream.frames = func(yield func(received, *provider.ProviderFailure) bool) {
		for data, err := range provider.SSEData(response.Body) {
			if ctx.Err() != nil {
				return
			}
			if err != nil {
				failure := provider.EventStreamFailure("Codex", err)
				yield(received{}, &failure)
				return
			}
			if strings.TrimSpace(data) == "[DONE]" {
				continue
			}
			known, err := provider.ResponsesEventKnown([]byte(data))
			if err != nil {
				failure := provider.Undecodable("Codex", err, data)
				yield(received{}, &failure)
				return
			}
			if known && !yield(received{data: data, record: data}, nil) {
				return
			}
		}
	}
	return stream, nil, nil
}

//demi:wire open
type envelope struct {
	Type     *string                    `json:"type,omitzero" check:"nullabsent"`
	Event    *map[string]jsontext.Value `json:"event,omitzero" check:"nullabsent"`
	Response *jsontext.Value            `json:"response,omitzero" check:"nullabsent"`
}

func decodeMessage(text string) (received, bool, error) {
	value, err := decode[envelope]([]byte(text))
	if err != nil {
		return received{}, false, err
	}
	data := text
	if value.Type != nil && *value.Type == "response.done" {
		response := jsontext.Value(`null`)
		if value.Response != nil {
			response = *value.Response
		}
		data = string(provider.JSONBody(map[string]jsontext.Value{"type": jsontext.Value(`"response.completed"`), "response": response}))
	} else if value.Event != nil {
		data = string(provider.JSONBody(*value.Event))
	}
	known, err := provider.ResponsesEventKnown([]byte(data))
	return received{data: data, record: text}, known, err
}

// handshakeTransport preserves the complete vendor refusal. websocket.Dial keeps
// only its first 1024 bytes; diagnostics must retain the vendor's whole response.
type handshakeTransport struct {
	base    http.RoundTripper
	refusal *refusal
}

func (t *handshakeTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	response, err := t.base.RoundTrip(request)
	if err != nil || response.StatusCode == http.StatusSwitchingProtocols {
		return response, err
	}
	raw, readErr := io.ReadAll(response.Body)
	response.Body.Close()
	if readErr != nil {
		raw = nil
	}
	t.refusal = &refusal{status: uint16(response.StatusCode), headers: response.Header.Clone(), body: strings.ToValidUTF8(string(raw), "\uFFFD")}
	response.Body = io.NopCloser(bytes.NewReader(raw))
	return response, nil
}

func (r *runtimeProvider) websocket(ctx context.Context, headers http.Header, body []byte) (*opened, *refusal, *provider.ProviderFailure) {
	headers = headers.Clone()
	headers.Del("Accept")
	headers.Del("Content-Type")
	headers.Set("Openai-Beta", "responses_websockets=2026-02-06")
	connectCtx, cancel := context.WithTimeout(ctx, r.shared.config.ConnectTimeout)
	client := *r.http
	capture := &handshakeTransport{base: client.Transport}
	if capture.base == nil {
		capture.base = http.DefaultTransport
	}
	client.Transport = capture
	socket, response, err := websocket.Dial(connectCtx, r.shared.websocketURL, &websocket.DialOptions{HTTPClient: &client, HTTPHeader: headers})
	timedOut := connectCtx.Err() == context.DeadlineExceeded
	cancel()
	if response != nil {
		r.shared.quota.Observe(provider.Observation{Status: uint16(response.StatusCode), Headers: response.Header})
	}
	if err != nil {
		if response != nil && response.StatusCode != 101 {
			return nil, capture.refusal, nil
		}
		failure := provider.NoAnswer(fmt.Sprintf("Codex WebSocket connect failed: %v", provider.WithoutEndpoint(err)))
		if timedOut {
			failure = provider.NoAnswer(fmt.Sprintf("Codex WebSocket connect timed out after %dms", r.shared.config.ConnectTimeout.Milliseconds()))
		}
		return nil, nil, &failure
	}
	socket.SetReadLimit(64 << 20)
	message := append([]byte(`{"type":"response.create",`), body[1:]...)
	if err := socket.Write(ctx, websocket.MessageText, message); err != nil {
		failure := provider.NoAnswer(fmt.Sprintf("Codex WebSocket send failed: %v", err))
		return &opened{socket: socket, close: func() { socket.CloseNow() }, frames: func(yield func(received, *provider.ProviderFailure) bool) { yield(received{}, &failure) }}, nil, nil
	}
	stream := &opened{socket: socket, close: func() { socket.CloseNow() }}
	stream.frames = func(yield func(received, *provider.ProviderFailure) bool) {
		cancelled := make(chan struct{})
		stop := context.AfterFunc(ctx, func() { socket.Close(websocket.StatusNormalClosure, "aborted"); close(cancelled) })
		defer func() {
			if !stop() {
				<-cancelled
			}
		}()
		for {
			var idle atomic.Bool
			var timer *time.Timer
			var idleDone chan struct{}
			if r.shared.config.StreamIdleTimeout != nil {
				idleDone = make(chan struct{})
				timer = time.AfterFunc(*r.shared.config.StreamIdleTimeout, func() { idle.Store(true); socket.Close(websocket.StatusNormalClosure, "idle_timeout"); close(idleDone) })
			}
			_, raw, err := socket.Read(context.WithoutCancel(ctx))
			if timer != nil && !timer.Stop() {
				<-idleDone
			}
			if ctx.Err() != nil {
				return
			}
			if idle.Load() {
				failure := provider.NoAnswer(fmt.Sprintf("Codex WebSocket stream idled for %dms", r.shared.config.StreamIdleTimeout.Milliseconds()))
				yield(received{}, &failure)
				return
			}
			if err != nil {
				if websocket.CloseStatus(err) != -1 {
					return
				}
				failure := provider.NoAnswer(fmt.Sprintf("Codex WebSocket failed: %v", err))
				yield(received{}, &failure)
				return
			}
			text := string(raw)
			if !utf8.Valid(raw) {
				failure := provider.ProtocolFailure("Codex API stream sent a binary message that is not UTF-8 text", strings.ToValidUTF8(text, "\uFFFD"))
				yield(received{}, &failure)
				return
			}
			frame, known, err := decodeMessage(text)
			if err != nil {
				failure := provider.Undecodable("Codex", err, text)
				yield(received{}, &failure)
				return
			}
			if known && !yield(frame, nil) {
				return
			}
		}
	}
	return stream, nil, nil
}
func (p *Provider) refused(r refusal) provider.ProviderFailure {
	failure := provider.Refused("Codex", r.status, r.headers, r.body, ReadFailure, p.clock.Now())
	var object map[string]jsontext.Value
	if json.Unmarshal([]byte(r.body), &object) == nil {
		var fields map[string]jsontext.Value
		if json.Unmarshal(object["error"], &fields) == nil {
			for _, key := range []string{"code", "type"} {
				var value provider.ReportedString
				if json.Unmarshal(fields[key], &value) == nil && value.Value != nil {
					failure.Diagnostics.ProviderCode = value.Value
					break
				}
			}
		}
		var requestID provider.ReportedString
		if json.Unmarshal(object["request_id"], &requestID) == nil {
			failure.Diagnostics.ProviderRequestID = requestID.Value
		}
	}
	if value := r.headers.Get("X-Request-Id"); value != "" {
		failure.Diagnostics.ProviderRequestID = &value
	}
	return failure
}
