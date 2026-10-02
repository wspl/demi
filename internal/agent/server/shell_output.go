package server

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
)

func (t *Tree[H]) shellGroup() (host.Declared, error) {
	shape := shellOutputContract()
	input, err := declare.NewSchema(shape.input)
	if err != nil {
		return host.Declared{}, err
	}
	leaf := declare.Leaf[declare.NativeOperation]{Name: "output", Summary: shellOutputSummary, Input: input, Positionals: new([]string{"id"}), Kind: &declare.RPC[declare.NativeOperation]{}, SuccessOutput: new("the page, the lines, or with --raw the bytes on stdout; with --raw, a line on stderr where bytes were left out"), FailureOutput: new(`"demi shell output: <reason>" on stderr, exit 1`)}
	handler := host.TypedRPC(shape.decode, func(ctx context.Context, call host.Call[outputArgs], port host.RPCPort) (uint8, error) {
		if call.Invocation.Caller == nil {
			return 0, &host.RPCError{Kind: host.HandlerFailed, Message: "the command runs only in an agent's job"}
		}
		root, err := core.ParseNodeID(call.Invocation.Context.Conversation)
		if err != nil {
			return 0, err
		}
		tree := t.server.Tree(root)
		if tree == nil {
			return 0, &host.RPCError{Kind: host.HandlerFailed, Message: fmt.Sprintf("the conversation %s is not open", root)}
		}
		return tree.output(ctx, call.Args, port)
	})
	return host.Group("shell", shellGroupSummary, host.Leaf(leaf, handler)), nil
}

func (t *Tree[H]) output(ctx context.Context, args outputArgs, port host.RPCPort) (uint8, error) {
	fail := func(reason string) (uint8, error) {
		return 1, port.Stderr(ctx, []byte("demi shell output: "+reason+"\n"))
	}
	streams := host.Both
	stdout, stderr := args.Stdout != nil && *args.Stdout, args.Stderr != nil && *args.Stderr
	if stdout && stderr {
		return fail("--stdout and --stderr do not go together")
	}
	if stdout {
		streams = host.OnlyStdout
	}
	if stderr {
		streams = host.OnlyStderr
	}
	if args.Lines != nil && args.Tail != nil {
		return fail("--lines and --tail do not go together")
	}
	from, to := uint64(1), ^uint64(0)
	if args.Lines != nil {
		left, right, _ := strings.Cut(*args.Lines, "-")
		var firstErr, lastErr error
		from, firstErr = strconv.ParseUint(left, 10, 64)
		to, lastErr = strconv.ParseUint(right, 10, 64)
		if firstErr != nil || lastErr != nil || from < 1 || to < from {
			return fail(fmt.Sprintf("--lines %s: lines count from 1, and a range ends at or after its start", *args.Lines))
		}
	}
	raw := args.Raw != nil && *args.Raw
	if raw && (args.Lines != nil || args.Tail != nil) {
		return fail("--raw prints all of the output; take part of it with sed or tail")
	}
	output, running, err := t.commandOutput(ctx, args.ID)
	if err != nil {
		return fail(err.Error())
	}
	if raw {
		text := output.Text(streams, nil, host.Seen{})
		data := text.Bytes()
		for len(data) != 0 {
			size := min(len(data), 1024*1024)
			if err := port.Stdout(ctx, data[:size]); err != nil {
				return 0, err
			}
			data = data[size:]
		}
		for _, note := range text.Notes() {
			if err := port.Stderr(ctx, []byte(note+"\n")); err != nil {
				return 0, err
			}
		}
		return 0, nil
	}
	var binary *uint64
	if length, ok := output.BinaryStdoutLength(); ok {
		binary = &length
	}
	if binary != nil && streams == host.OnlyStdout {
		return fail(fmt.Sprintf("the stdout of %s is binary; save it: demi shell output %s --raw --stdout > <file>", args.ID, args.ID))
	}
	text := output.Text(streams, binary, host.Seen{})
	page := outputPage{text: text, id: args.ID, streams: streams, running: running}
	if args.Tail != nil {
		return commandOut(ctx, port, page.tail(*args.Tail))
	}
	if args.Lines != nil && from > text.LastLine() {
		return fail(fmt.Sprintf("lines %d-%d are past the end: the output has %d lines", from, to, text.LastLine()))
	}
	return commandOut(ctx, port, page.forward(from, min(to, text.LastLine())))
}

func (t *Tree[H]) commandOutput(ctx context.Context, id string) (host.WholeOutput, bool, error) {
	unknown := fmt.Errorf("no command %s in this conversation", id)
	command, err := core.ParseCommandID(id)
	if err != nil {
		return host.WholeOutput{}, false, unknown
	}
	node := t.shellsOf(command)
	if environment := node.runtime.access.Environments.Owning(command); environment != nil {
		output, err := environment.ReadOutput(ctx, command)
		if err == nil {
			return output, true, nil
		}
		var shell *host.ShellError
		if !errors.As(err, &shell) || shell.Kind != host.NotRunning {
			return host.WholeOutput{}, false, fmt.Errorf("the output of %s could not be read: %w", id, err)
		}
	}
	stored, err := t.store.CommandOutput(ctx, command)
	if err != nil {
		return host.WholeOutput{}, false, fmt.Errorf("the output of %s could not be read: %w", id, err)
	}
	switch v := stored.(type) {
	case *store.OutputStored:
		return v.Output, false, nil
	case *store.OutputNotStored:
		return host.WholeOutput{}, false, fmt.Errorf("the output of %s was not stored: %s", id, v.Reason)
	case *store.OutputRemoved:
		return host.WholeOutput{}, false, fmt.Errorf("the output of %s was removed on %s, %d days after the command ended", id, string(v.At)[:10], store.CommandOutputDays)
	}
	return host.WholeOutput{}, false, unknown
}

type outputPage struct {
	text    host.OutputText
	id      string
	streams host.Streams
	running bool
}

func (p outputPage) flags() string {
	switch p.streams {
	case host.OnlyStdout:
		return " --stdout"
	case host.OnlyStderr:
		return " --stderr"
	default:
		return ""
	}
}

func (p outputPage) header(first, last uint64) string {
	streams := "stdout and stderr"
	if p.streams == host.OnlyStdout {
		streams = "stdout"
	}
	if p.streams == host.OnlyStderr {
		streams = "stderr"
	}
	soFar := ""
	if p.running {
		soFar = " so far"
	}
	if first != 0 {
		return fmt.Sprintf("[command %s: lines %d-%d of %d%s, %s]", p.id, first, last, p.text.LastLine(), soFar, streams)
	}
	return fmt.Sprintf("[command %s: %d lines%s, %s]", p.id, p.text.LastLine(), soFar, streams)
}

func (p outputPage) reserved(to uint64) int {
	return utf8.RuneCountInString(p.header(to, to)) + utf8.RuneCountInString(fmt.Sprintf("[next: demi shell output %s --lines %d-%d%s]", p.id, to, to, p.flags())) + 2
}

func (p outputPage) render(piece host.Piece) []string {
	text := piece.Text()
	if piece.Number == 0 {
		return []string{text}
	}
	length := utf8.RuneCountInString(text)
	if length <= 2000 {
		return []string{fmt.Sprintf("%6d\t%s", piece.Number, text)}
	}
	return []string{fmt.Sprintf("%6d\t%s", piece.Number, text[:core.CharOffset(text, 2000)]), fmt.Sprintf("[line %d is %d characters; whole: demi shell output %s --raw%s | sed -n %dp]", piece.Number, length, p.id, p.flags(), piece.Number)}
}

func (p outputPage) forward(from, to uint64) string {
	budget := tools.PageChars - p.reserved(to)
	lines := []string{}
	used, first, last, next := 0, uint64(0), uint64(0), uint64(0)
	for piece := range p.text.Forward(from) {
		if piece.Number > to {
			break
		}
		rendered := p.render(piece)
		cost := 0
		for _, line := range rendered {
			cost += utf8.RuneCountInString(line) + 1
		}
		if used+cost > budget && len(lines) != 0 {
			next = piece.Number
			break
		}
		used += cost
		lines = append(lines, rendered...)
		if piece.Number != 0 {
			if first == 0 {
				first = piece.Number
			}
			last = piece.Number
		}
	}
	page := append([]string{p.header(first, last)}, lines...)
	if next != 0 {
		page = append(page, fmt.Sprintf("[next: demi shell output %s --lines %d-%d%s]", p.id, next, to, p.flags()))
	}
	return strings.Join(page, "\n") + "\n"
}

func (p outputPage) tail(count uint64) string {
	lastLine := p.text.LastLine()
	from := uint64(1)
	if lastLine >= count {
		from = lastLine - count + 1
	}
	budget, used := tools.PageChars-p.reserved(lastLine), 0
	first, last := uint64(0), uint64(0)
	lines := [][]string{}
	for piece := range p.text.Backward() {
		if piece.Number != 0 && piece.Number < from {
			break
		}
		rendered := p.render(piece)
		cost := 0
		for _, line := range rendered {
			cost += utf8.RuneCountInString(line) + 1
		}
		if used+cost > budget && len(lines) != 0 {
			break
		}
		used += cost
		lines = append(lines, rendered)
		if piece.Number != 0 {
			if last == 0 {
				last = piece.Number
			}
			first = piece.Number
		}
	}
	slices.Reverse(lines)
	page := []string{p.header(first, last)}
	for _, line := range lines {
		page = append(page, line...)
	}
	return strings.Join(page, "\n") + "\n"
}
