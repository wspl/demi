package usershard

import (
	"context"
	"fmt"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapi"
)

const switched = "[Execution target switched]"
const cloudReset = "Cloud was reset: system packages and configuration were rebuilt from the base image. Files under /home remain. Running processes, temporary files, and previous shell state are gone; check the environment before continuing."

func (s *Shard) executionContext(ctx context.Context, id webapi.ConversationID, seen []string) (*string, error) {
	control := s.Control()
	record, err := control.Conversation(ctx, id)
	if err != nil || record == nil {
		return nil, err
	}
	if record.ContextVersion == 0 {
		return nil, nil
	}
	marker := fmt.Sprintf("[Execution context %d]", record.ContextVersion)
	if slices.ContainsFunc(seen, func(text string) bool { return strings.Contains(text, marker) }) {
		return nil, nil
	}
	attached, err := control.AttachedHosts(ctx, id)
	if err != nil {
		return nil, err
	}
	lines := []string{marker}
	reset, err := control.AnnouncedCloudReset(ctx, id)
	if err != nil {
		return nil, err
	}
	if reset != nil {
		marker := fmt.Sprintf("[Cloud reset %s]", *reset)
		if !slices.ContainsFunc(seen, func(text string) bool { return strings.Contains(text, marker) }) {
			lines = append(lines, marker, cloudReset)
		}
	}
	change, err := control.LastSwitch(ctx, id)
	if err != nil {
		return nil, err
	}
	announced := false
	if change != nil {
		before, err := s.describeTarget(ctx, change.From)
		if err != nil {
			return nil, err
		}
		after, err := s.describeTarget(ctx, change.To)
		if err != nil {
			return nil, err
		}
		description := fmt.Sprintf("Previous target: %s. Current target: %s. New shells start in %s.", before, after, database.ExecutionPath(change.To))
		observed := false
		for i := len(seen) - 1; i >= 0; i-- {
			if strings.Contains(seen[i], switched) {
				observed = strings.Contains(seen[i], description)
				break
			}
		}
		if !observed {
			lines = append(lines, switchLines(*change, description, attached)...)
			announced = true
		}
	}
	if !announced {
		lines = append(lines, "[Attached hosts changed]")
	}
	lines = append(lines, s.attachedHostsLine(attached))
	text := strings.Join(lines, "\n")
	return &text, nil
}
func (s *Shard) describeTarget(ctx context.Context, target database.ExecutionTarget) (string, error) {
	device := database.ExecutionDeviceID(target)
	if device == nil {
		return "Cloud (not allocated)", nil
	}
	name := string(*device)
	record, err := s.Control().Device(ctx, *device)
	if err != nil {
		return "", err
	}
	if record != nil {
		name = "\"" + record.Name + "\""
	}
	if target, ok := target.(*database.ExecutionWorkspace); ok {
		workspace := string(target.WorkspaceID)
		record, err := s.Control().Workspace(ctx, target.WorkspaceID)
		if err != nil {
			return "", err
		}
		if record != nil {
			workspace = record.Name
		}
		return fmt.Sprintf("workspace \"%s\" — directory %s on device %s", workspace, target.Path, name), nil
	}
	return fmt.Sprintf("the machine %s (host %s)", name, *device), nil
}
func (s *Shard) attachedHostsLine(attached []database.AttachedHostRecord) string {
	const list = "`demi host list` shows every host this conversation can reach."
	if len(attached) == 0 {
		return "Attached hosts: none. " + list
	}
	entries := make([]string, 0, len(attached))
	for _, host := range attached {
		state := "offline"
		if s.devices.Online(host.Device) {
			state = "online"
		}
		directory := "its home directory"
		if host.CWD != nil {
			directory = *host.CWD
		} else if home, ok := s.devices.Home(host.Device); ok {
			directory = home
		}
		entries = append(entries, fmt.Sprintf("\"%s\" (%s, shells start in %s)", host.Name, state, directory))
	}
	return "Attached hosts: " + strings.Join(entries, ", ") + ". `demi host shell --host <name> <script>` runs a shell string on one; " + list
}
func switchLines(change database.TargetSwitch, description string, attached []database.AttachedHostRecord) []string {
	lines := []string{switched, description, "No files were moved: everything created earlier lives on the previous target, and file paths from before the switch — including the full outputs of earlier commands — are stale here."}
	if device := database.ExecutionDeviceID(change.From); device != nil {
		for _, departed := range attached {
			if departed.Device == *device {
				name, from := departed.Name, database.ExecutionPath(change.From)
				lines = append(lines, fmt.Sprintf("The previous host stays attached as \"%s\": `demi host shell --host %s <script>` runs a shell string there with byte-faithful stdio, starting in %s (e.g. `demi host shell --host %s \"tar c -C %s .\" | tar x` pulls its files into the current directory).", name, name, from, name, from))
				break
			}
		}
	}
	before, ok := change.From.(*database.ExecutionWorkspace)
	after, also := change.To.(*database.ExecutionWorkspace)
	if ok && also && before.DeviceID == after.DeviceID {
		lines = append(lines, fmt.Sprintf("The previous directory %s is on the same device, so it is also directly accessible from this shell.", before.Path))
	}
	return lines
}
