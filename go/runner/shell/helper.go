package shell

import (
	"encoding/json/v2"
	"errors"
	"fmt"
	"os"
)

const helperFlag = "--demi-shell-exec"

type childLimit struct {
	Resource   int
	Soft, Hard uint64
}
type childSetup struct {
	JobHandle uintptr
	Path      string
	Args      []string
	Mask      *uint32
	Limits    []childLimit
	Mode      string
}

// ExecHelper must be called at the start of the runner's main, before serving
// connections. It returns for ordinary invocations; private child invocations
// validate their arguments, set process attributes, and replace themselves.
// Tests use the same entry point from TestMain.
func ExecHelper() {
	if len(os.Args) < 2 || os.Args[1] != helperFlag {
		return
	}
	var setup childSetup
	var err error
	if len(os.Args) != 3 {
		err = errors.New("invalid exec helper arguments")
	} else {
		err = json.Unmarshal([]byte(os.Args[2]), &setup, json.RejectUnknownMembers(true))
	}
	if err == nil && setup.Mask != nil && *setup.Mask > 0777 {
		err = errors.New("invalid child umask")
	}
	if err == nil && setup.Mode != "exec" && setup.Mode != "probe" && setup.Mode != "umask" {
		err = errors.New("invalid exec helper mode")
	}
	if err == nil && setup.Mode == "exec" && (setup.Path == "" || len(setup.Args) == 0) {
		err = errors.New("missing child program")
	}
	if err == nil {
		err = executeHelper(setup)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	os.Exit(0)
}

func helperArgs(setup childSetup) (string, []string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", nil, err
	}
	if setup.Mode == "" {
		setup.Mode = "exec"
	}
	data, err := json.Marshal(setup)
	if err != nil {
		return "", nil, err
	}
	return exe, []string{exe, helperFlag, string(data)}, nil
}
