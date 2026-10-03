// Command release packages and publishes Demi's programs, images and records.
package main

import (
	"context"
	"flag"
	"fmt"
	"io"
	"os"
	"os/signal"
)

const help = `The repository's development commands

native build [--package EXECUTABLE] [--target TRIPLE] [--artifacts DIRECTORY]
  Compiles executables for their targets into an artifact directory.
native package --package EXECUTABLE --output DIRECTORY [--target TRIPLE] [--artifacts DIRECTORY] [--resources DIRECTORY]
  Turns built executables into a release directory.
cloud-image package --root DIRECTORY --runners DIRECTORY --package DIRECTORY --output DIRECTORY
  Completes the Cloud image release of the tree rootfs/build.sh made, and publishes it.
browser-release VERSION
  Pins a Chrome for Testing version: writes its release record.
fork diff [--patch]
  Lists the shell fork's changes from its upstream release.
dev [--port 3271] [--keep]
  Runs a development backend with a scripted Cloud and an echo model.
`

type stringsFlag []string

func (s *stringsFlag) String() string     { return fmt.Sprint([]string(*s)) }
func (s *stringsFlag) Set(v string) error { *s = append(*s, v); return nil }

type buildOptions struct {
	Packages, Targets stringsFlag
	Artifacts         string
}
type packageOptions struct {
	Package, Output, Artifacts, Resources string
	Targets                               stringsFlag
}
type imageOptions struct {
	Root, Runners, Output string
	Packages              stringsFlag
}
type devOptions struct {
	Port uint
	Keep bool
}
type application struct {
	Root     string
	Out, Err io.Writer
}

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), stopSignals()...)
	defer stop()
	root, err := repository()
	if err == nil {
		err = (&application{root, os.Stdout, os.Stderr}).run(ctx, os.Args[1:])
	}
	if err != nil {
		_, _ = fmt.Fprintln(os.Stderr, "release:", err)
		os.Exit(1)
	}
}
func repository() (string, error) { panic("not written: t-release") }
func (a *application) run(ctx context.Context, args []string) error {
	if len(args) == 0 || args[0] == "--help" || args[0] == "-h" {
		_, err := io.WriteString(a.Out, help)
		return err
	}
	f := flag.NewFlagSet(args[0], flag.ContinueOnError)
	f.SetOutput(a.Err)
	switch args[0] {
	case "native":
		if len(args) < 2 {
			return fmt.Errorf("native requires build or package")
		}
		if args[1] == "build" {
			var o buildOptions
			f.Var(&o.Packages, "package", "An executable to build; repeat for several [default: the runner and the command programs].")
			f.Var(&o.Targets, "target", "A target to build for; repeat for several [default: every target of each executable].")
			f.StringVar(&o.Artifacts, "artifacts", "", "The artifact directory the builds write [default: .cache/native-target in the repository].")
			if err := f.Parse(args[2:]); err != nil {
				return err
			}
			if f.NArg() != 0 {
				return fmt.Errorf("unexpected arguments: %v", f.Args())
			}
			return a.build(ctx, o)
		}
		if args[1] == "package" {
			var o packageOptions
			f.StringVar(&o.Package, "package", "", "The executable to package.")
			f.StringVar(&o.Output, "output", "", "The release directory; for the runner, the directory of its releases.")
			f.StringVar(&o.Artifacts, "artifacts", "", "The artifact directory the build wrote [default: .cache/native-target in the repository].")
			f.StringVar(&o.Resources, "resources", "", "Where the resource archives are kept, each named by its SHA-256 [default: .cache/resources in the repository].")
			f.Var(&o.Targets, "target", "A target to package; repeat for several [default: every target of the executable].")
			if err := f.Parse(args[2:]); err != nil {
				return err
			}
			if o.Package == "" || o.Output == "" || f.NArg() != 0 {
				return fmt.Errorf("native package requires --package and --output")
			}
			return a.packageNative(ctx, o)
		}
	case "cloud-image":
		if len(args) < 2 || args[1] != "package" {
			return fmt.Errorf("cloud-image requires package")
		}
		var o imageOptions
		f.StringVar(&o.Root, "root", "", "The Ubuntu tree rootfs/build.sh made.")
		f.StringVar(&o.Runners, "runners", "", "The runner releases; the image embeds the one their manifest names.")
		f.StringVar(&o.Output, "output", "", "The release directory to publish, a new one for every build.")
		f.Var(&o.Packages, "package", "A command package release to embed; repeat for each package.")
		if err := f.Parse(args[2:]); err != nil {
			return err
		}
		if o.Root == "" || o.Runners == "" || o.Output == "" || len(o.Packages) == 0 || f.NArg() != 0 {
			return fmt.Errorf("cloud-image package requires --root, --runners, --package and --output")
		}
		return a.image(ctx, o)
	case "browser-release":
		if err := f.Parse(args[1:]); err != nil {
			return err
		}
		if f.NArg() != 1 {
			return fmt.Errorf("browser-release requires Chrome's four-part version, such as 153.0.8010.36")
		}
		return a.browser(ctx, f.Arg(0))
	case "fork":
		if len(args) < 2 || args[1] != "diff" {
			return fmt.Errorf("fork requires diff")
		}
		patch := f.Bool("patch", false, "Prints the unified diff instead of the summary.")
		if err := f.Parse(args[2:]); err != nil {
			return err
		}
		if f.NArg() != 0 {
			return fmt.Errorf("unexpected arguments: %v", f.Args())
		}
		return a.fork(ctx, *patch)
	case "dev":
		var o devOptions
		f.UintVar(&o.Port, "port", 3271, "The port the backend listens on.")
		f.BoolVar(&o.Keep, "keep", false, "Keep the data directory when the command ends.")
		if err := f.Parse(args[1:]); err != nil {
			return err
		}
		if o.Port > 65535 || f.NArg() != 0 {
			return fmt.Errorf("invalid dev arguments")
		}
		return a.dev(ctx, o)
	}
	return fmt.Errorf("unknown command: %v", args)
}
