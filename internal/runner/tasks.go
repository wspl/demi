package runner

import (
	"maps"

	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runnerproto"
)

// taskEnvironment supplies the installation-owned defaults for backend work.
type taskEnvironment struct {
	values       map[string]string
	cwd          string
	home         string
	installation string
}

// shellSpec combines the device and requested environment, keeping installation authority local.
func (e taskEnvironment) shellSpec(request *runnerproto.JobStart) jobs.TaskSpec {
	values := maps.Clone(e.values)
	if values == nil {
		values = make(map[string]string)
	}
	maps.Copy(values, request.Env)
	if _, exists := values["HOME"]; !exists {
		values["HOME"] = e.home
	}
	values["DEMI_HOME"] = e.installation
	command := &jobs.ShellCommand{Script: request.Script, Stdin: request.Stdin, Stdout: request.Stdout}
	if request.ManifestHash != nil {
		command.Commands = &jobs.DeclaredCommands{ManifestHash: *request.ManifestHash, Context: request.Context}
	}
	return jobs.TaskSpec{ID: request.JobID, Cwd: request.CWD, Env: values, Command: command}
}

// processSpec applies raw-process environment replacement, overlays and removal.
func (e taskEnvironment) processSpec(request *runnerproto.Spawn) jobs.TaskSpec {
	values := make(map[string]string)
	if request.Env == nil || request.InheritEnv != nil && *request.InheritEnv {
		maps.Copy(values, e.values)
	}
	if request.Env != nil {
		for name, value := range *request.Env {
			if value == nil {
				delete(values, name)
			} else {
				values[name] = *value
			}
		}
	}
	values["DEMI_HOME"] = e.installation
	cwd := e.cwd
	if request.CWD != nil {
		cwd = *request.CWD
	}
	var args []string
	if request.Args != nil {
		args = *request.Args
	}
	command := &jobs.ProcessCommand{
		Command:      request.Command,
		Args:         args,
		ProcessGroup: request.KillProcessGroup != nil && *request.KillProcessGroup,
	}
	return jobs.TaskSpec{ID: request.SpawnID, Cwd: cwd, Env: values, Command: command}
}
