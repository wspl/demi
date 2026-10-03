package server

import (
	"context"
	"fmt"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
)

type telemetry struct {
	lastEvent int64
	lastText  *int64
	tools     []toolRecord
}
type toolRecord struct {
	id      string
	title   string
	started int64
	ended   *int64
	status  toolStatus
}

// observe records only the tool and text events a supervisor may show.
func (t *Tree[H]) observe(c *child[H], event *session.TranscriptChanged) {
	now, err := t.server.deps.Clock.Now().Millisecond()
	if err != nil {
		t.report(err)
		return
	}
	textIndices := map[uint32]bool{}
	for _, patch := range event.Patches {
		if p, ok := patch.(*framewire.AppendTextPatch); ok {
			textIndices[p.Index] = c.node.session.IsTextAt(int(p.Index))
		}
	}
	t.server.mu.Lock()
	defer t.server.mu.Unlock()
	for _, patch := range event.Patches {
		switch p := patch.(type) {
		case *framewire.AddPatch:
			c.telemetry.addBlockLocked(p.Value, now)
		case *framewire.AppendTextPatch:
			if textIndices[p.Index] {
				c.telemetry.lastText = new(now)
				c.telemetry.lastEvent = now
			}
		case *framewire.ReplaceBlockPatch:
			c.telemetry.replaceBlockLocked(p.Value, now)
		case *framewire.ReplacePatch:
		}
	}
}

func (t *Tree[H]) snapshot(c *child[H], parent uint64, now int64) (agentSnapshot, error) {
	record := c.node.record
	began, err := record.StartedAt.Millisecond()
	if err != nil {
		return agentSnapshot{}, err
	}
	execution := c.node.session.Execution()
	text := boundedResult(c.node.session.LastAssistantText())
	t.server.mu.Lock()
	telemetry := c.telemetry
	telemetry.tools = slices.Clone(telemetry.tools)
	t.server.mu.Unlock()
	since, activity := telemetry.lastEvent, string(execution)
	if execution == session.ProviderStreaming {
		activity = "streaming"
	}
	if execution == session.ToolExecuting {
		for i := len(telemetry.tools) - 1; i >= 0; i-- {
			if tool := telemetry.tools[i]; tool.ended == nil {
				since, activity = tool.started, tool.title
				break
			}
		}
	}
	result := agentSnapshot{
		SubagentID:        record.Number,
		ParentSessionID:   parent,
		Description:       record.Description,
		Profile:           record.Profile,
		Phase:             framewire.JobPhaseRunning,
		ElapsedMS:         agentAge(now, began),
		LastEventMS:       agentAge(now, telemetry.lastEvent),
		Execution:         execution,
		Activity:          activity,
		ExecutionForMS:    agentAge(now, since),
		Tools:             []toolSnapshot{},
		LastAssistantText: text,
	}
	if telemetry.lastText != nil {
		result.LastAssistantTextAgoMS = new(agentAge(now, *telemetry.lastText))
	}
	for _, tool := range telemetry.tools {
		end := now
		var ago *uint64
		if tool.ended != nil {
			end = *tool.ended
			ago = new(agentAge(now, end))
		}
		result.Tools = append(
			result.Tools,
			toolSnapshot{
				Title:      tool.title,
				Status:     tool.status,
				DurationMS: agentAge(end, tool.started),
				EndedAgoMS: ago,
			},
		)
	}
	return result, nil
}

// agentAge expresses supervisor ages without negative clock differences.
func agentAge(now, then int64) uint64 {
	if now < then {
		return 0
	}
	return uint64(now) - uint64(then)
}

// agentDuration uses the compact, rounded durations of list and show.
func agentDuration(ms uint64) string {
	seconds := (ms + 500) / 1000
	if seconds < 60 {
		return fmt.Sprintf("%ds", seconds)
	}
	minutes := seconds / 60
	if minutes < 60 {
		if seconds%60 == 0 {
			return fmt.Sprintf("%dm", minutes)
		}
		return fmt.Sprintf("%dm%ds", minutes, seconds%60)
	}
	if minutes%60 == 0 {
		return fmt.Sprintf("%dh", minutes/60)
	}
	return fmt.Sprintf("%dh%dm", minutes/60, minutes%60)
}

func showText(s agentSnapshot) string {
	description, profile := s.Description, "(inherit)"
	if description == "" {
		description = "(none)"
	}
	if s.Profile != nil {
		profile = *s.Profile
	}
	lines := []string{
		fmt.Sprintf("id: %d", s.SubagentID),
		fmt.Sprintf("parent: %d", s.ParentSessionID),
		"description: " + description,
		"profile: " + profile,
		"phase: " + string(s.Phase),
		"elapsed: " + agentDuration(s.ElapsedMS),
		fmt.Sprintf("execution: %s (for %s)", s.Execution, agentDuration(s.ExecutionForMS)),
		"last-event: " + agentDuration(s.LastEventMS) + " ago",
		"activity: " + s.Activity,
	}
	if len(s.Tools) > 0 {
		lines = append(lines, fmt.Sprintf("recent tool calls (last %d):", len(s.Tools)))
		for _, tool := range s.Tools {
			if tool.EndedAgoMS == nil {
				lines = append(
					lines,
					fmt.Sprintf("  [executing for %s] %s", agentDuration(tool.DurationMS), tool.Title),
				)
			} else {
				lines = append(
					lines,
					fmt.Sprintf(
						"  [%s in %s, ended %s ago] %s",
						tool.Status,
						agentDuration(tool.DurationMS),
						agentDuration(*tool.EndedAgoMS),
						tool.Title,
					),
				)
			}
		}
	}
	if s.LastAssistantTextAgoMS != nil && s.LastAssistantText != "" {
		lines = append(
			lines,
			"last assistant text ("+agentDuration(*s.LastAssistantTextAgoMS)+" ago):",
			s.LastAssistantText,
		)
	} else {
		lines = append(lines, "last assistant text: (none yet)")
	}
	return strings.Join(lines, "\n") + "\n"
}

func showCommand[H host.Host](
	ctx context.Context,
	t *Tree[H],
	_ core.NodeID,
	jsonOutput bool,
	args showArgs,
	port host.RPCPort,
) (uint8, error) {
	for _, c := range t.descendants(t.id) {
		if c.node.record.Number != args.ID {
			continue
		}
		parent := t.Node(*c.node.record.Parent)
		if parent == nil {
			break
		}
		now, err := t.server.deps.Clock.Now().Millisecond()
		if err != nil {
			return 0, err
		}
		snapshot, err := t.snapshot(c, parent.record.Number, now)
		if err != nil {
			return 0, err
		}
		if jsonOutput {
			return commandJSON(ctx, port, shown{Agent: snapshot})
		}
		return commandOut(ctx, port, showText(snapshot))
	}
	return commandFail(ctx, port, "show", fmt.Errorf("no live agent %d", args.ID))
}

type listNode struct {
	entry    treeEntry
	id       core.NodeID
	line     string
	children []listNode
}

func (t *Tree[H]) listNode(
	ctx context.Context,
	node *Node[H],
	c *child[H],
	parent *uint64,
	caller core.NodeID,
	now int64,
) (listNode, error) {
	record := node.record
	result := listNode{
		id: record.ID,
		entry: treeEntry{
			SubagentID:      record.Number,
			ParentSessionID: parent,
			Kind:            "root",
			Description:     record.Description,
			Profile:         record.Profile,
			Phase:           framewire.JobPhaseRunning,
			Self:            record.ID == caller,
		},
	}
	if c != nil {
		result.entry.Kind = "live"
		snapshot, err := t.snapshot(c, *parent, now)
		if err != nil {
			return listNode{}, err
		}
		result.line = liveAgentLine(record, snapshot)
	}
	for _, live := range t.childrenOf(record.ID) {
		child, err := t.listNode(ctx, live.node, live, new(record.Number), caller, now)
		if err != nil {
			return listNode{}, err
		}
		result.children = append(result.children, child)
	}
	archived, err := t.archivedNodes(ctx, node, caller, now)
	if err != nil {
		return listNode{}, err
	}
	result.children = append(result.children, archived...)
	return result, nil
}

func quotedDescription(text string) string {
	if text == "" {
		return "(no description)"
	}
	return `"` + text + `"`
}

func (n listNode) render(prefix string, last bool, lines *[]string, entries *[]treeEntry) {
	*entries = append(*entries, n.entry)
	marker := ""
	if n.entry.Self {
		marker = " ← you"
	}
	body := fmt.Sprintf("● %d  (root session)%s", n.entry.SubagentID, marker)
	if n.entry.Kind == "live" {
		body = "● " + n.line + marker
	}
	if n.entry.Kind == "archived" {
		body = fmt.Sprintf(
			"○ %d  archived (%s %s ago)  %s",
			n.entry.SubagentID,
			n.entry.Phase,
			agentDuration(*n.entry.ClosedAgoMS),
			quotedDescription(n.entry.Description),
		)
	}
	childPrefix := ""
	if n.entry.Kind != "root" {
		branch, continuation := "├─", "│ "
		if last {
			branch, continuation = "└─", "  "
		}
		body = prefix + branch + body
		childPrefix = prefix + continuation
	}
	*lines = append(*lines, body)
	for i, child := range n.children {
		child.render(childPrefix, i+1 == len(n.children), lines, entries)
	}
}

func listCommand[H host.Host](
	ctx context.Context,
	t *Tree[H],
	caller core.NodeID,
	jsonOutput bool,
	_ listArgs,
	port host.RPCPort,
) (uint8, error) {
	now, err := t.server.deps.Clock.Now().Millisecond()
	if err != nil {
		return commandFail(ctx, port, "list", err)
	}
	root, err := t.listNode(ctx, t.root, nil, nil, caller, now)
	if err != nil {
		return commandFail(ctx, port, "list", err)
	}
	lines, entries := []string{}, []treeEntry{}
	root.render("", true, &lines, &entries)
	if jsonOutput {
		return commandJSON(ctx, port, listing{Tree: entries})
	}
	return commandOut(ctx, port, strings.Join(lines, "\n")+"\n")
}

// toolTitle uses the model's concrete description when the call carries one.
func toolTitle(name, input string) string {
	call, err := decodeTitledCall([]byte(input))
	if err == nil && !core.IsBlank(call.Description) {
		return core.Trim(call.Description)
	}
	return name
}

// addBlockLocked updates supervisor telemetry while the server mutex is held.
func (t *telemetry) addBlockLocked(value core.Block, now int64) {
	switch block := value.(type) {
	case *core.ToolCallBlock:
		title := toolTitle(block.ToolName, block.Input)
		t.tools = append(
			t.tools,
			toolRecord{id: block.ToolUseID, title: title, started: now, status: "executing"},
		)
		for len(t.tools) > 8 {
			i := slices.IndexFunc(t.tools, func(tool toolRecord) bool { return tool.ended != nil })
			if i < 0 {
				break
			}
			t.tools = slices.Delete(t.tools, i, i+1)
		}
		t.lastEvent = now
	case *core.TextBlock:
		t.lastText = new(now)
		t.lastEvent = now
	case *core.AbortBlock,
		*core.AgentMessageBlock,
		*core.CompactionBoundaryBlock,
		*core.CompactionMarkerBlock,
		*core.ContextBlock,
		*core.ErrorBlock,
		*core.RedactedThinkingBlock,
		*core.ResponseBlock,
		*core.ResumeBlock,
		*core.SteerBlock,
		*core.ThinkingBlock,
		*core.UserBlock,
		*core.WakeupBlock:
	}
}

// replaceBlockLocked records a completed tool while the server mutex is held.
func (t *telemetry) replaceBlockLocked(value core.Block, now int64) {
	if call, ok := value.(*core.ToolCallBlock); ok && call.Status != core.ToolCallStatusExecuting {
		for i := range t.tools {
			record := &t.tools[i]
			if record.id == call.ToolUseID && record.ended == nil {
				record.ended = new(now)
				record.status = "completed"
				if call.Status == core.ToolCallStatusError {
					record.status = "error"
				}
				break
			}
		}
		t.lastEvent = now
	}
}

func liveAgentLine(record store.NodeRecord, snapshot agentSnapshot) string {
	profile := "(inherit)"
	if snapshot.Profile != nil {
		profile = *snapshot.Profile
	}
	return strings.Join(
		[]string{
			fmt.Sprint(record.Number),
			string(snapshot.Phase),
			"up " + agentDuration(snapshot.ElapsedMS),
			"last-event " + agentDuration(snapshot.LastEventMS) + " ago",
			"profile=" + profile,
			quotedDescription(record.Description),
			"execution=" + string(snapshot.Execution),
			"activity=" + snapshot.Activity,
		},
		"  ",
	)
}

func (t *Tree[H]) archivedNodes(ctx context.Context, node *Node[H], caller core.NodeID, now int64) ([]listNode, error) {
	children := []listNode{}
	records, err := t.store.Children(ctx, node.record.ID)
	if err != nil {
		return nil, err
	}
	archived := slices.DeleteFunc(
		records,
		func(record store.NodeRecord) bool { return record.Closed == nil || t.Node(record.ID) != nil },
	)
	slices.SortStableFunc(
		archived,
		func(a, b store.NodeRecord) int { return strings.Compare(string(b.Closed.At), string(a.Closed.At)) },
	)
	for _, record := range archived {
		at, err := record.Closed.At.Millisecond()
		if err != nil {
			return nil, err
		}
		children = append(
			children,
			listNode{
				id: record.ID,
				entry: treeEntry{
					SubagentID:      record.Number,
					ParentSessionID: new(node.record.Number),
					Kind:            "archived",
					Description:     record.Description,
					Profile:         record.Profile,
					Phase:           record.Closed.Phase.JobPhase(),
					ClosedAgoMS:     new(agentAge(now, at)),
					Self:            record.ID == caller,
				},
			},
		)
	}
	return children, nil
}
