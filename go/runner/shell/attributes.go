package shell

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"strconv"
	"strings"

	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

// Child attributes live in unexported interpreter variables so that mvdan
// copies them with subshell state. No process-wide mask or limit is changed.
const maskVariable = "__demi_child_umask"

const limitOptions = "bcdefiklmnpqrstuvxPRT"

func limitVariable(side string, option rune) string {
	return fmt.Sprintf("__demi_limit_%s_%c", side, option)
}

func setAttribute(ctx context.Context, name, value string) error {
	quoted, err := syntax.Quote(value, syntax.LangBash)
	if err != nil {
		return err
	}
	return interp.HandlerCtx(ctx).Builtin(ctx, []string{"eval", name + "=" + quoted})
}

// childAttributes reads the subshell's private child settings.
func childAttributes(ctx context.Context) (childSetup, error) {
	env := interp.HandlerCtx(ctx).Env
	var setup childSetup
	if mask := env.Get(maskVariable).String(); mask != "" {
		value, err := strconv.ParseUint(mask, 8, 32)
		if err != nil {
			return setup, err
		}
		converted := uint32(value)
		setup.Mask = &converted
	}
	for _, option := range limitOptions {
		resource, ok := resourceNumbers[option]
		if !ok {
			continue
		}
		soft := env.Get(limitVariable("S", option)).String()
		if soft == "" {
			continue
		}
		hard := env.Get(limitVariable("H", option)).String()
		lo, err := strconv.ParseUint(soft, 10, 64)
		if err != nil {
			return setup, err
		}
		hi, err := strconv.ParseUint(hard, 10, 64)
		if err != nil {
			return setup, err
		}
		setup.Limits = append(setup.Limits, childLimit{resource, lo, hi})
	}
	return setup, nil
}

func probeAttributes(ctx context.Context, setup childSetup) (string, error) {
	if setup.Mode != "umask" {
		setup.Mode = "probe"
	}
	path, args, err := helperArgs(setup)
	if err != nil {
		return "", err
	}
	var stdout, stderr bytes.Buffer
	err = launch(ctx, path, args, &stdout, &stderr)
	if err != nil {
		if stderr.Len() != 0 {
			return "", errors.New(strings.TrimSpace(stderr.String()))
		}
		// Rust validates whether setting the limit succeeds, not whether the
		// disposable probe survives it (a zero CPU limit can kill the probe).
		var status interp.ExitStatus
		if setup.Mode != "umask" && errors.As(err, &status) {
			return "", nil
		}
		return "", err
	}
	return stdout.String(), nil
}

func capitalize(s string) string {
	if s == "" {
		return s
	}
	return strings.ToUpper(s[:1]) + s[1:]
}

func usage(ctx context.Context, name, message string) error {
	if _, err := fmt.Fprintf(interp.HandlerCtx(ctx).Stderr, "%s: %s\n", name, message); err != nil {
		return err
	}
	return interp.ExitStatus(2)
}

func umask(ctx context.Context, args []string) error {
	reusable, symbolic := false, false
	var mode *string
	for _, arg := range args[1:] {
		if mode == nil && len(arg) > 1 && strings.HasPrefix(arg, "-") && strings.Trim(strings.TrimPrefix(arg, "-"), "pS") == "" {
			reusable = reusable || strings.Contains(arg, "p")
			symbolic = symbolic || strings.Contains(arg, "S")
		} else if mode == nil {
			value := arg
			mode = &value
		} else {
			return usage(ctx, "umask", fmt.Sprintf("%s: too many arguments", arg))
		}
	}
	stored := interp.HandlerCtx(ctx).Env.Get(maskVariable).String()
	var mask uint64
	if stored != "" {
		value, err := strconv.ParseUint(stored, 8, 32)
		if err != nil {
			return err
		}
		mask = value
	} else {
		output, err := probeAttributes(ctx, childSetup{Mode: "umask"})
		if err != nil {
			return err
		}
		value, err := strconv.ParseUint(strings.TrimSpace(output), 10, 32)
		if err != nil {
			return err
		}
		mask = value
	}
	if mode != nil {
		value, err := parseMask(uint32(mask), *mode)
		if err != nil {
			return diagnostic(ctx, "umask", fmt.Errorf("%s: %w", *mode, err))
		}
		mask = uint64(value)
	}
	if mode != nil {
		if err := setAttribute(ctx, maskVariable, fmt.Sprintf("%04o", mask)); err != nil {
			return err
		}
		if !symbolic {
			return nil
		}
	}
	shown := fmt.Sprintf("%04o", mask)
	if symbolic {
		var groups []string
		for i, who := range []string{"u", "g", "o"} {
			allowed := ^mask >> ((2 - i) * 3) & 7
			var permissions strings.Builder
			for j, letter := range "rwx" {
				if allowed&(4>>j) != 0 {
					permissions.WriteRune(letter)
				}
			}
			groups = append(groups, who+"="+permissions.String())
		}
		shown = strings.Join(groups, ",")
	}
	if reusable {
		if symbolic {
			shown = "umask -S " + shown
		} else {
			shown = "umask " + shown
		}
	}
	_, err := fmt.Fprintln(interp.HandlerCtx(ctx).Stdout, shown)
	return err
}

// builtinUsage emits the Rust parser's complete diagnostic, including spacing.
func builtinUsage(ctx context.Context, text string) error {
	if _, err := fmt.Fprint(interp.HandlerCtx(ctx).Stderr, text); err != nil {
		return err
	}
	return interp.ExitStatus(2)
}
