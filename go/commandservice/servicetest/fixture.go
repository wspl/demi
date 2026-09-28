// Package servicetest supplies the shared native fixture handler.
package servicetest

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"math"
	"os"
	"slices"
	"strconv"
	"strings"
	"sync"

	cs "github.com/wspl/demi/go/commandservice"
)

// FixtureOperations returns the fixture catalog in the Rust fixture's order.
func FixtureOperations() []string {
	return []string{
		"where", "echo", "first", "spin", "result", "retain",
		"stall_release", "held", "crash", "stalled", "proceed", "number",
	}
}

// Fixture implements the Rust native-test fixture’s operations and retained conversations.
type Fixture struct {
	mu               sync.Mutex
	held, stalling   map[string]struct{}
	stalled, proceed chan struct{}
}

// NewFixture creates an empty native-test fixture with its coordination signals.
func NewFixture() *Fixture {
	return &Fixture{
		held:     make(map[string]struct{}),
		stalling: make(map[string]struct{}),
		stalled:  make(chan struct{}, 1),
		proceed:  make(chan struct{}, 1),
	}
}

// Operations returns the fixture operation catalog in wire order.
func (f *Fixture) Operations() []string { return FixtureOperations() }

// notify retains one fixture coordination event until its operation consumes it.
func notify(c chan struct{}) {
	select {
	case c <- struct{}{}:
	default:
	}
}

func (f *Fixture) heldNames() []string {
	names := make([]string, 0, len(f.held))
	for n := range f.held {
		names = append(names, n)
	}
	slices.Sort(names)
	return names
}

// Invoke executes one native fixture operation with the Rust fixture’s outputs and failures.
func (f *Fixture) Invoke(c *cs.Call) (cs.Completion, error) {
	v := c.Invocation
	var err error
	completion := cs.Completion{}
	switch v.Operation {
	case "number":
		var args map[string]jsontext.Value
		if err := json.Unmarshal(v.Args, &args); err != nil {
			return completion, err
		}
		count := uint64(1)
		if parsed, err := strconv.ParseUint(strings.TrimSpace(string(args["count"])), 10, 64); err == nil {
			count = parsed
		}
		if count > math.MaxUint32 {
			return completion, errors.New("out of range integral type conversion attempted")
		}
		var first uint64
		first, err = c.Numbers.Draw(c.Context(), v.Context.Conversation, cs.Tab, int(count))
		if err == nil {
			err = json.MarshalWrite(c.Stdout, map[string]any{"first": first})
		}
	case "stalled":
		select {
		case <-f.stalled:
		case <-c.Context().Done():
			err = c.Context().Err()
		}
	case "proceed":
		notify(f.proceed)
	case "retain", "stall_release":
		f.mu.Lock()
		f.held[v.Context.Conversation] = struct{}{}
		if v.Operation == "stall_release" {
			f.stalling[v.Context.Conversation] = struct{}{}
		}
		f.mu.Unlock()
	case "held":
		f.mu.Lock()
		names := f.heldNames()
		f.mu.Unlock()
		err = json.MarshalWrite(c.Stdout, cs.ConversationStatus{Conversations: names})
	case "crash":
		// A broken diagnostic pipe cannot prevent the fixture’s deliberate process exit.
		fmt.Fprintln(os.Stderr, "fixture crashing on purpose")
		os.Exit(3)
	case "where":
		var value any
		if s, ok := v.Env["PROBE"]; ok {
			value = s
		}
		var args map[string]jsontext.Value
		if err = json.Unmarshal(v.Args, &args); err != nil {
			return completion, err
		}
		answer := struct {
			Label   jsontext.Value    `json:"label"`
			Context cs.CommandContext `json:"context"`
			Cwd     string            `json:"cwd"`
			Value   any               `json:"value"`
		}{Label: args["label"], Context: v.Context, Cwd: v.Cwd, Value: value}
		err = json.MarshalWrite(c.Stdout, answer, json.Deterministic(true))
	case "echo", "first":
		for {
			var b []byte
			b, err = c.Stdin.Next()
			if err == io.EOF {
				err = nil
				break
			}
			if err != nil {
				break
			}
			_, err = c.Stdout.Write(b)
			if err != nil || v.Operation == "first" {
				break
			}
		}
	case "spin":
		if _, err = c.Stdout.Write([]byte("started")); err == nil {
			//lint:ignore SA5002 The fixture deliberately reproduces Rust’s noncooperative CPU-bound spin.
			for {
			}
		}
	case "result":
		_, err = c.Stdout.Write([]byte("command output"))
		if err == nil {
			_, err = c.Stderr.Write([]byte("command diagnostic"))
		}
		if err == nil && v.Env["RESULT"] == "error" {
			err = errors.New("command failed")
		}
		completion.ExitCode = 17
	default:
		err = fmt.Errorf("unknown operation %s", v.Operation)
	}
	return completion, err
}

// Conversation reports or releases retained fixture state, including its deliberate failure cases.
func (f *Fixture) Conversation(c *cs.ConversationCall) (cs.Completion, error) {
	req := c.Request
	f.mu.Lock()
	if _, stalling := f.stalling[req.Conversation]; req.Operation == "release" && stalling {
		f.mu.Unlock()
		notify(f.stalled)
		<-c.Context().Done()
		return cs.Completion{}, c.Context().Err()
	}
	var value any
	stall := false
	var err error
	if req.Operation == "status" {
		if _, unanswerable := f.held["unanswerable"]; unanswerable {
			err = errors.New("fixture status unavailable")
		} else {
			value = cs.ConversationStatus{Conversations: f.heldNames()}
			_, stall = f.held["stall"]
		}
	} else if req.Conversation == "fail" {
		err = errors.New("fixture cleanup failed")
	} else {
		delete(f.held, req.Conversation)
		value = struct{}{}
	}
	f.mu.Unlock()
	if err != nil {
		return cs.Completion{}, err
	}
	if stall {
		notify(f.stalled)
		select {
		case <-f.proceed:
		case <-c.Context().Done():
			return cs.Completion{}, c.Context().Err()
		}
	}
	return cs.Completion{}, json.MarshalWrite(c.Stdout, value)
}
