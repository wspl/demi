package expose

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
)

// run answers a command after its declaration has checked the invocation.
func run(ctx context.Context, invocation host.RPCInvocation, port plugin.Port) (uint8, error) {
	var text, refusal string
	var err error
	leaf := ""
	if len(invocation.Path) > 0 {
		leaf = invocation.Path[len(invocation.Path)-1]
	}
	switch leaf {
	case "add":
		var args AddArgs
		args, err = commandArgs(invocation, DecodeAddArgs)
		if err == nil {
			text, refusal, err = add(ctx, args, invocation.JSON, port)
		}
	case "list":
		text, err = list(ctx, invocation.JSON, port)
	case "renew", "remove":
		var args NumberArgs
		args, err = commandArgs(invocation, DecodeNumberArgs)
		if err == nil {
			text, refusal, err = change(ctx, leaf, args.Number, invocation.JSON, port)
		}
	default:
		return 0, &plugin.ErrorFailed{Message: "no such expose command"}
	}
	if err != nil {
		return 0, plugin.RequestError(err)
	}
	if refusal != "" {
		err = port.RPC().Stderr(ctx, []byte(fmt.Sprintf("expose %s: %s\n", leaf, refusal)))
		return 1, err
	}
	return 0, port.RPC().Stdout(ctx, []byte(text))
}

func add(ctx context.Context, args AddArgs, jsonOutput bool, port plugin.Port) (string, string, error) {
	hosts, err := port.ConversationHosts(ctx)
	if err != nil {
		return "", "", err
	}
	var target *plugin.ConversationHost
	for i := range hosts {
		h := &hosts[i]
		if args.Host == nil && h.Role == plugin.HostRoleMain || args.Host != nil && h.Name == *args.Host {
			target = h
			break
		}
	}
	if target == nil && args.Host != nil {
		for i := range hosts {
			if string(hosts[i].Device) == *args.Host {
				target = &hosts[i]
				break
			}
		}
	}
	if target == nil {
		wanted := "(main)"
		if args.Host != nil {
			wanted = *args.Host
		}
		return "", fmt.Sprintf("host %s is not reachable from this conversation (see `demi host list`)", wanted), nil
	}
	created, err := port.CreateExpose(ctx, target.Device, args.Address, lifetime)
	refused, err := exposeRefusal(err)
	if err != nil || refused != "" {
		return "", refused, err
	}
	listed, err := port.Exposes(ctx)
	if err != nil {
		return "", "", err
	}
	entries, err := numbered(ctx, port, listed.Exposes)
	if err != nil {
		return "", "", err
	}
	for _, entry := range entries {
		if entry.expose.ID != created.ID {
			continue
		}
		if jsonOutput {
			data, err := (ExposeAnswer{Expose: line(entry)}).MarshalJSON()
			return string(data), "", err
		}
		return fmt.Sprintf("Exposed %s on %s as %s\nExpires in %d minutes (expose %d).\n", entry.expose.Address, target.Name, entry.expose.URL, lifetime/60, entry.number), "", nil
	}
	return "", "the expose ended at once", nil
}

func list(ctx context.Context, jsonOutput bool, port plugin.Port) (string, error) {
	listed, err := port.Exposes(ctx)
	if err != nil {
		return "", err
	}
	entries, err := numbered(ctx, port, listed.Exposes)
	if err != nil {
		return "", err
	}
	if jsonOutput {
		lines := make([]ExposeLine, 0, len(entries))
		for _, entry := range entries {
			lines = append(lines, line(entry))
		}
		data, err := (ExposeLines{Exposes: lines}).MarshalJSON()
		return string(data), err
	}
	if len(entries) == 0 {
		return "No exposes.\n", nil
	}
	now, err := listed.ListedAt.Millisecond()
	if err != nil {
		return "", err
	}
	rows := [][5]string{{"Expose", "Device", "Address", "Expires", "URL"}}
	for _, entry := range entries {
		expiry, err := entry.expose.ExpiresAt.Millisecond()
		if err != nil {
			return "", err
		}
		left := max(int64(0), expiry-now)
		rows = append(rows, [5]string{fmt.Sprint(entry.number), entry.expose.DeviceName, string(entry.expose.Address), fmt.Sprintf("%d min", (left+30000)/60000), entry.expose.URL})
	}
	return table(rows), nil
}

func change(ctx context.Context, method string, number uint64, jsonOutput bool, port plugin.Port) (string, string, error) {
	listed, err := port.Exposes(ctx)
	if err != nil {
		return "", "", err
	}
	entries, err := numbered(ctx, port, listed.Exposes)
	if err != nil {
		return "", "", err
	}
	missing := fmt.Sprintf("no expose %d", number)
	for _, entry := range entries {
		if entry.number != number {
			continue
		}
		if method == "remove" {
			refused, err := exposeRefusal(port.RemoveExpose(ctx, entry.expose.ID))
			if err != nil {
				return "", "", err
			}
			if refused != "" {
				return "", missing, nil
			}
			return fmt.Sprintf("Removed expose %d; its URL no longer works.\n", number), "", nil
		}
		renewed, err := port.RenewExpose(ctx, entry.expose.ID, lifetime)
		refused, err := exposeRefusal(err)
		if err != nil {
			return "", "", err
		}
		if refused != "" {
			return "", missing, nil
		}
		if jsonOutput {
			data, err := (ExposeAnswer{Expose: line(numberedExpose{number: number, expose: renewed})}).MarshalJSON()
			return string(data), "", err
		}
		return fmt.Sprintf("Expose %d expires in %d minutes.\n", number, lifetime/60), "", nil
	}
	return "", missing, nil
}

func exposeRefusal(err error) (string, error) {
	var refused *plugin.PortRefusalExpose
	if !errors.As(err, &refused) {
		return "", err
	}
	switch refused.Reason {
	case plugin.ExposeRefusalUnavailable:
		return "exposes are not available on this instance", nil
	case plugin.ExposeRefusalDeviceOffline:
		return "the device is offline; connect it before exposing a service", nil
	case plugin.ExposeRefusalInvalidAddress, plugin.ExposeRefusalDeviceNotFound, plugin.ExposeRefusalNotFound:
		return refused.Message, nil
	}
	return "", err
}

func line(entry numberedExpose) ExposeLine {
	e := entry.expose
	return ExposeLine{Number: entry.number, Device: e.DeviceName, Address: string(e.Address), URL: e.URL, ExpiresAt: e.ExpiresAt}
}

// table formats expose rows at Unicode character widths, two spaces apart.
// No existing table formatter is provided by this package's allowed dependencies.
func table(rows [][5]string) string {
	var widths [5]int
	for _, row := range rows {
		for i, cell := range row {
			widths[i] = max(widths[i], utf8.RuneCountInString(cell))
		}
	}
	var text strings.Builder
	for _, row := range rows {
		for i, cell := range row {
			text.WriteString(cell)
			if i < 4 {
				text.WriteString(strings.Repeat(" ", widths[i]-utf8.RuneCountInString(cell)+2))
			}
		}
		text.WriteByte('\n')
	}
	return text.String()
}

// commandArgs decodes the declared expose input and identifies the command on failure.
func commandArgs[T any](invocation host.RPCInvocation, decode func([]byte) (T, error)) (T, error) {
	value, err := decode(invocation.Args)
	if err != nil {
		return value, fmt.Errorf("the arguments of %q do not decode as declared: %w", strings.Join(invocation.Path, " "), err)
	}
	return value, nil
}
