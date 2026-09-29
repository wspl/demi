package backendtest

import (
	"net/http"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest/scripted"
)

// Anthropic creates the session's API-key entry whose endpoint is the vendor
// under prefix, so that conversations on entries of their own get their own
// answers however their requests interleave, and answers its id.
func (b *Backend) Anthropic(session *Session, vendor *scripted.Vendor, prefix string) string {
	b.t.Helper()
	body := Map{
		"source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "sk-ant-test",
		"baseUrl": vendor.URL(prefix + "/v1"),
	}
	return b.Post("/api/providers", session, body).Expect(http.StatusCreated).Str("provider.id")
}

// CreateConversation creates a conversation under id, as a page's first action.
func (b *Backend) CreateConversation(session *Session, id string) *Answer {
	b.t.Helper()
	return b.Post("/api/conversations", session, Map{"id": id}).Expect(http.StatusCreated)
}

// Choose chooses the model of the entry provider for the conversation, as a
// page's first send does, and answers the conversation.
func (b *Backend) Choose(session *Session, id, provider, model string) *Answer {
	b.t.Helper()
	body := Map{"model": Map{"providerId": provider, "modelId": model}}
	return b.Patch("/api/conversations/"+id, session, body).Expect(http.StatusOK)
}

// SwitchTo moves the conversation to path on the device, as the page's target
// switch does. A switch refuses running work, so it follows a turn whose end
// the socket has seen.
func (b *Backend) SwitchTo(session *Session, id string, device *Paired, path string) {
	b.t.Helper()
	target := Map{"target": Map{"kind": "device", "deviceId": device.ID(), "path": path}}
	b.Patch("/api/conversations/"+id, session, target).Expect(http.StatusOK)
}

// Summary returns the conversation's summary as the sidebar lists it. Once a
// socket has seen the idle phase, the save that ended the turn has committed
// (runtime.md § A turn), so the summary holds the turn.
func (b *Backend) Summary(session *Session, id string) map[string]any {
	b.t.Helper()
	listed, _ := b.Get("/api/conversations", session).Expect(http.StatusOK).At("conversations").([]any)
	for _, conversation := range listed {
		if summary, ok := conversation.(map[string]any); ok && summary["id"] == id {
			return summary
		}
	}
	b.t.Fatalf("the conversation %s is not listed", id)
	return nil
}

// Transcript returns the conversation's blocks as the transcript route reads
// them.
func (b *Backend) Transcript(session *Session, id string) []any {
	b.t.Helper()
	blocks, _ := b.Get("/api/conversations/"+id+"/transcript", session).Expect(http.StatusOK).At("blocks").([]any)
	return blocks
}

// BlockKinds returns each block's type.
func BlockKinds(blocks []any) []string {
	kinds := make([]string, len(blocks))
	for index, block := range blocks {
		kinds[index], _ = At(block, "type").(string)
	}
	return kinds
}

// A Driven is a conversation a scenario drives: its socket, and the path of the
// scripted endpoint its model answers from.
type Driven struct {
	t      testing.TB
	vendor *scripted.Vendor
	route  string
	// Socket is the conversation's socket.
	Socket *Socket

	// seen are the tool calls whose results were read already.
	seen map[string]bool
	sent int
}

// Open opens the conversation id with the model of provider, whose endpoint is
// under prefix.
func (b *Backend) Open(session *Session, vendor *scripted.Vendor, id, provider, prefix string) *Driven {
	b.t.Helper()
	d := &Driven{t: b.t, vendor: vendor, route: prefix + "/v1/messages", seen: map[string]bool{}}
	d.Socket = d.connect(b, session, id, provider)
	return d
}

func (d *Driven) connect(b *Backend, session *Session, id, provider string) *Socket {
	d.t.Helper()
	b.Choose(session, id, provider, "claude-opus-4-8")
	socket := b.Connect(session, id)
	socket.Open()
	return socket
}

// Reconnect opens the conversation again, as a reload after a restart does; the
// tool results read already stay read.
func (d *Driven) Reconnect(b *Backend, session *Session, id, provider string) {
	d.t.Helper()
	d.Socket = d.connect(b, session, id, provider)
}

// Start scripts the model's answers and sends a message, without waiting for its
// turn; it answers how many requests the vendor had before.
func (d *Driven) Start(answers ...*scripted.Response) int {
	d.t.Helper()
	before := len(d.vendor.Requests())
	for _, response := range answers {
		d.vendor.RespondAt(d.route, response)
	}
	d.sent++
	d.Socket.Send(SendMessage(messageID(d.sent), "go"))
	return before
}

// messageID names the nth message a scenario sends.
func messageID(n int) string {
	return "m" + strconv.Itoa(n)
}

// A Turn is a turn as the model saw it.
type Turn struct {
	// Received is each tool result the model received, in order.
	Received []string
	// Requests are the requests the turn made.
	Requests []any
}

// FirstRequest is the text of the turn's first request's messages, context
// blocks included.
func (t Turn) FirstRequest() string {
	return string(Marshal(At(t.Requests[0], "messages")))
}

// Turn scripts the model's answers, sends a message and waits for its turn to
// end.
func (d *Driven) Turn(answers ...*scripted.Response) Turn {
	d.t.Helper()
	before := d.Start(answers...)
	d.Socket.UntilIdle()
	return d.Observe(before)
}

// Observe returns what the requests since the before'th showed the model.
func (d *Driven) Observe(before int) Turn {
	d.t.Helper()
	var turn Turn
	for _, request := range d.vendor.Requests()[before:] {
		if request.Path != d.route {
			continue
		}
		body := request.JSON(d.t)
		turn.Requests = append(turn.Requests, body)
		messages, _ := At(body, "messages").([]any)
		for _, message := range messages {
			content, _ := At(message, "content").([]any)
			for _, block := range content {
				if At(block, "type") != "tool_result" {
					continue
				}
				id, _ := At(block, "tool_use_id").(string)
				if d.seen[id] {
					continue
				}
				d.seen[id] = true
				var text []string
				parts, _ := At(block, "content").([]any)
				for _, part := range parts {
					if piece, ok := At(part, "text").(string); ok {
						text = append(text, piece)
					} else {
						text = append(text, "[image]")
					}
				}
				turn.Received = append(turn.Received, strings.Join(text, "\n"))
			}
		}
	}
	return turn
}

// ShellCall is the model's shell call id running script, watched for at most
// timeout.
func ShellCall(id, script string, timeout time.Duration) *scripted.Response {
	return scripted.ToolUse(id, "shell_exec", Map{"description": id, "script": script, "timeoutMs": timeout.Milliseconds()})
}

// Say is the model's closing words.
func Say(text string) *scripted.Response {
	return scripted.Answer([]string{text}, 1, 1)
}

// Field is the value of a shell tool result's "name: value" line, such as its
// commandId (runtime.md § Results and previews).
func Field(t testing.TB, result, name string) string {
	t.Helper()
	for line := range strings.SplitSeq(result, "\n") {
		if value, ok := strings.CutPrefix(line, name+": "); ok {
			return value
		}
	}
	t.Fatalf("the result has no %s:\n%s", name, result)
	return ""
}

// ShownOutput is the output a shell tool result shows: what the command wrote
// since the model's last look, each line with its newline; empty when it shows
// none. A running command's lines after it, its newest output's line and its
// next step, are not part of it.
func ShownOutput(result string) string {
	_, output, found := strings.Cut(result, "\noutput:\n")
	if !found {
		return ""
	}
	output, _, _ = strings.Cut(output, "\nnext: ")
	if at := strings.Index(output, " bytes not shown so far; the newest: "); at >= 0 {
		shown := ""
		if cut := strings.LastIndex(output[:at], "\n[... "); cut >= 0 {
			shown = output[:cut]
		}
		output = shown
	}
	return output + "\n"
}
