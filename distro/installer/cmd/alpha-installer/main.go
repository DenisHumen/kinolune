// SPDX-License-Identifier: MPL-2.0

// Command alpha-installer is the Kinolune OS installer.
//
//	alpha-installer disks [--json] [--all] [--smart] [--from DIR] [--record DIR]
//	alpha-installer install --answers FILE [--dry-run] [--yes] [--from DIR]
//
// `disks` is read-only (DIST-004). `install` wipes one disk and installs the
// OS non-interactively from an answer file (DIST-005); the TUI comes in DIST-006.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
	"github.com/DenisHumen/kinolune/distro/installer/internal/install"
)

// version is set at build time: -ldflags "-X main.version=…".
var version = "dev"

const usage = `usage:
  alpha-installer disks [--json] [--all] [--smart] [--from DIR] [--record DIR]
  alpha-installer install --answers FILE [--dry-run] [--yes] [--from DIR]`

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if err := run(ctx, os.Args[1:], os.Stdin, os.Stdout, os.Stderr); err != nil {
		fmt.Fprintln(os.Stderr, "alpha-installer:", err)
		os.Exit(1)
	}
}

func run(ctx context.Context, args []string, stdin io.Reader, stdout, stderr io.Writer) error {
	if len(args) == 0 {
		return errors.New(usage)
	}
	switch args[0] {
	case "disks":
		return runDisks(args[1:], stdout)
	case "install":
		return runInstall(ctx, args[1:], stdin, stdout, stderr, liveEnv())
	case "version", "--version":
		fmt.Fprintln(stdout, version)
		return nil
	}
	return errors.New(usage)
}

func runDisks(args []string, stdout io.Writer) error {
	fs := flag.NewFlagSet("disks", flag.ContinueOnError)
	asJSON := fs.Bool("json", false, "print JSON instead of a table")
	all := fs.Bool("all", false, "also list excluded devices (boot media, loop, zram, optical…)")
	smart := fs.Bool("smart", false, "query SMART health (smartctl -H) for candidate disks")
	from := fs.String("from", "", "read a recorded snapshot directory instead of the live system")
	record := fs.String("record", "", "save a redacted snapshot of this machine to DIR and exit")
	if err := fs.Parse(args); err != nil {
		return err
	}

	var src disks.Source = disks.System{}
	if *from != "" {
		src = disks.Dir(*from)
	}
	if *record != "" {
		return disks.Record(src, *record)
	}
	list, err := disks.Probe(src, disks.Options{Smart: *smart})
	if err != nil {
		return err
	}
	if *asJSON {
		if !*all {
			list = disks.Candidates(list)
		}
		enc := json.NewEncoder(stdout)
		enc.SetIndent("", "  ")
		return enc.Encode(list)
	}
	return disks.RenderText(stdout, list, *all)
}

// env is what runInstall needs from the machine; tests replace it.
type env struct {
	runner  install.Runner
	resolve func(string) (string, error)
	isTTY   func() bool
	isRoot  func() bool
	live    disks.Source
}

func liveEnv() env {
	return env{
		runner:  install.ExecRunner{Log: os.Stderr},
		resolve: filepath.EvalSymlinks,
		isTTY: func() bool {
			st, err := os.Stdin.Stat()
			return err == nil && st.Mode()&os.ModeCharDevice != 0
		},
		isRoot: func() bool { return os.Geteuid() == 0 },
		live:   disks.System{},
	}
}

func runInstall(ctx context.Context, args []string, stdin io.Reader, stdout, stderr io.Writer, e env) error {
	fs := flag.NewFlagSet("install", flag.ContinueOnError)
	fs.SetOutput(stderr)
	var answersPath string
	fs.StringVar(&answersPath, "answers", "", "answer file (JSON, see distro/installer/README.md)")
	fs.StringVar(&answersPath, "answer", "", "alias of --answers")
	dryRun := fs.Bool("dry-run", false, "validate and print the plan; touch nothing")
	yes := fs.Bool("yes", false, "wipe the disk from the answer file without asking")
	from := fs.String("from", "", "with --dry-run: plan against a recorded disk snapshot")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() > 0 {
		return fmt.Errorf("unexpected argument %q", fs.Arg(0))
	}
	if answersPath == "" {
		return errors.New("install needs --answers FILE (the interactive installer comes with DIST-006)")
	}
	if *from != "" && !*dryRun {
		return errors.New("--from works only with --dry-run")
	}

	f, err := os.Open(answersPath)
	if err != nil {
		return err
	}
	a, err := answers.Load(f)
	f.Close()
	if err != nil {
		return err
	}

	src := e.live
	resolve := e.resolve
	if *from != "" {
		src = disks.Dir(*from)
		resolve = nil // a snapshot has no /dev/disk/by-id symlinks
	}
	list, err := disks.Probe(src, disks.Options{})
	if err != nil {
		return err
	}
	target, err := install.ResolveTarget(a.Disk.Path, list, resolve)
	if err != nil {
		return err
	}
	in, err := install.New(a, target, e.runner, stdout)
	if err != nil {
		return err
	}
	in.Version = version
	if err := in.WritePlan(stdout); err != nil {
		return err
	}
	if *dryRun {
		fmt.Fprintln(stdout, "\nDry run: nothing was written.")
		return nil
	}
	if !e.isRoot() {
		return errors.New("the installation must run as root")
	}
	if !*yes {
		if !e.isTTY() {
			return fmt.Errorf("refusing to wipe %s without confirmation: add --yes", target.Path)
		}
		fmt.Fprintf(stdout, "\nType the disk name %q to erase it and install, anything else to cancel: ", target.Name)
		line, _ := bufio.NewReader(stdin).ReadString('\n')
		if strings.TrimSpace(line) != target.Name {
			return errors.New("cancelled, nothing was written")
		}
	}
	return in.Run(ctx)
}
