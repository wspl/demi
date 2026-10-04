package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
)

const (
	systemPrompt             = "You are a careful coding assistant. Remember any secrets the user told you verbatim."
	recallPrompt             = "只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:我最早让你记住的三个暗号分别是什么?"
	fillerPrompt             = "忽略下列填充并只回复 ok。"
	turnTimeout              = 600 * time.Second
	root         core.NodeID = "compaction-fixture"
)

type conversation struct {
	server     *server.Server[*toolstest.NoHost]
	connection *server.Connection[*toolstest.NoHost]
	frames     *server.Frames
	http       *http.Client
}

func openConversation(ctx context.Context, f fixture, model core.ModelSelection) (*conversation, error) {
	entry, err := deepseek()
	if err != nil {
		return nil, err
	}
	memory := storetest.NewMemoryTreeStore()
	initial := initialCheckpoint(f, model)
	if err := memory.CreateNode(ctx, store.RootRecord(root, core.SystemClock{}.Now()), initial); err != nil {
		return nil, err
	}
	client := &http.Client{}
	config := server.DefaultConfig()
	config.Session.Compaction.ThresholdPercent = 80
	s := server.New(server.Deps[*toolstest.NoHost]{
		Toolsets:     tools.Set{Commands: &host.CommandSet{}, Revision: "none"},
		Instructions: systemPrompt, Hosts: &toolstest.NoHost{}, Shells: toolstest.NoShells{},
		Providers: &deepSeek{provider: entry, http: client, selection: model},
		Stores:    func(core.NodeID) store.Tree { return memory },
		Clock:     core.SystemClock{}, IDs: transcript.RandomIDs{}, Config: config,
		StatusChanged: func(core.NodeID) {},
	})
	connection, frames := s.Connect(root, f.CWD, nil)
	c := &conversation{server: s, connection: connection, frames: frames, http: client}
	connection.Handle(ctx, &framewire.OpenFrame{})
	err = c.nextUntil(ctx, func(frame framewire.ServerFrame) bool {
		_, ok := frame.(*framewire.PendingSteersFrame)
		return ok
	})
	if err != nil {
		return nil, errors.Join(err, c.close(context.WithoutCancel(ctx)))
	}
	return c, nil
}

func (c *conversation) close(ctx context.Context) error {
	c.connection.Detach()
	err := c.server.Shutdown(ctx)
	c.http.CloseIdleConnections()
	return err
}

func (c *conversation) blocks() []core.Block {
	return c.server.Tree(root).Root().Session().Transcript().Blocks
}

func (c *conversation) generations() int {
	count := 0
	for _, block := range c.blocks() {
		if _, ok := block.(*core.CompactionBoundaryBlock); ok {
			count++
		}
	}
	return count
}

func (c *conversation) errors() int {
	count := 0
	for _, block := range c.blocks() {
		if _, ok := block.(*core.ErrorBlock); ok {
			count++
		}
	}
	return count
}

func (c *conversation) context(window uint32) (uint64, error) {
	blocks := c.blocks()
	start := transcript.ReplayStart(blocks)
	view, missing := store.NewModelView(start, blocks[start:], store.HeldMedia{})
	if len(missing) != 0 {
		return 0, errors.New("the fixture's history holds no media")
	}
	return transcript.Estimate(transcript.NewRequestView(view, flash(window).Model, provider.RequestLimits{})), nil
}

func (c *conversation) nextUntil(ctx context.Context, until func(framewire.ServerFrame) bool) error {
	ctx, cancel := context.WithTimeout(ctx, turnTimeout)
	defer cancel()
	for c.frames.Next(ctx) {
		if until(c.frames.Frame()) {
			return nil
		}
	}
	if err := c.frames.Err(); err != nil {
		return err
	}
	return io.EOF
}

func (c *conversation) act(ctx context.Context, frame framewire.ClientFrame) error {
	c.connection.Handle(ctx, frame)
	if err := c.nextUntil(ctx, func(frame framewire.ServerFrame) bool {
		phase, ok := frame.(*framewire.PhaseFrame)
		return ok && phase.Phase != core.SessionPhaseIdle
	}); err != nil {
		return err
	}
	return c.nextUntil(ctx, func(frame framewire.ServerFrame) bool {
		phase, ok := frame.(*framewire.PhaseFrame)
		return ok && phase.Phase == core.SessionPhaseIdle
	})
}

func (c *conversation) send(ctx context.Context, text string) (string, error) {
	before := len(c.blocks())
	id, err := core.ParseTurnID(transcript.RandomIDs{}.NextID())
	if err != nil {
		return "", err
	}
	err = c.act(ctx, &framewire.SendFrame{
		MessageID: id,
		Content:   []framewire.ClientContent{&framewire.TextContent{Text: text}},
	})
	if err != nil {
		return "", err
	}
	var texts []string
	for _, block := range c.blocks()[before:] {
		if text, ok := block.(*core.TextBlock); ok {
			texts = append(texts, text.Text)
		}
	}
	return strings.Join(texts, " "), nil
}

func (c *conversation) recall(ctx context.Context) (int, error) {
	answer, err := c.send(ctx, recallPrompt)
	if err != nil {
		return 0, err
	}
	count := recalled(answer)
	fmt.Printf("   recall %d/3: %s\n", count, tail(answer))
	return count, nil
}

func (c *conversation) grow(ctx context.Context, label string, chars int) error {
	_, err := c.send(ctx, fmt.Sprintf("%s\n\n%s-%s", fillerPrompt, label, strings.Repeat("x", chars)))
	return err
}

func (c *conversation) actSwitch(ctx context.Context, model core.ModelSelection) error {
	change, err := c.server.PrepareSwitch(ctx, root, model)
	if err != nil {
		return err
	}
	if change == nil {
		return errors.New("the conversation is open")
	}
	return c.server.SwitchModel(ctx, root, *change)
}

func recalled(answer string) int {
	answer = strings.ToUpper(answer)
	count := 0
	for _, secret := range [][2]string{{"ZEBRA", "7"}, {"QUARTZ", "9"}, {"NIMBUS", "3"}} {
		if strings.Contains(answer, secret[0]+"-"+secret[1]) || strings.Contains(answer, secret[0]+secret[1]) {
			count++
		}
	}
	return count
}

func tail(answer string) string {
	runes := []rune(answer)
	return string(runes[max(0, len(runes)-120):])
}
