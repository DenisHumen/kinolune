// SPDX-License-Identifier: MPL-2.0

package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"syscall"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
	"github.com/DenisHumen/kinolune/distro/installer/internal/install"
	"github.com/DenisHumen/kinolune/distro/installer/internal/tui"
)

// tuiEnv connects the TUI to the machine: the same probe, safety checks and
// installer as `alpha-installer install --answers`.
type tuiEnv struct {
	e        env
	src      disks.Source
	resolve  func(string) (string, error)
	dryRun   bool
	smart    bool
	sysRoot  string // root of /sys/class/net
	lockPath string
	logPath  string
	livePath string // answer file in the live system
	saveMnt  string
	snapshot bool // disks come from a recorded snapshot (--from)
}

func (t *tuiEnv) Disks() ([]disks.Disk, error) {
	return disks.Probe(t.src, disks.Options{Smart: t.smart})
}

func (t *tuiEnv) Interfaces() []tui.Iface { return tui.ReadInterfaces(t.sysRoot) }

func (t *tuiEnv) Install(ctx context.Context, a *answers.Answers, ui io.Writer) (string, error) {
	log := ui
	if t.logPath != "" {
		if err := os.MkdirAll(filepath.Dir(t.logPath), 0o700); err == nil {
			if f, err := os.OpenFile(t.logPath, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o600); err == nil {
				defer f.Close()
				log = io.MultiWriter(ui, f)
			}
		}
	}
	// Validate again: the answers come from the screens, but the rules are
	// those of the answer file.
	if err := a.Validate(); err != nil {
		return "", err
	}
	list, err := disks.Probe(t.src, disks.Options{})
	if err != nil {
		return "", err
	}
	target, err := install.ResolveTarget(a.Disk.Path, list, t.resolve)
	if err != nil {
		return "", err
	}
	in, err := install.New(a, target, quiet(t.e.runner, log), log)
	if err != nil {
		return "", err
	}
	in.Version = version
	if err := in.WritePlan(log); err != nil {
		return "", err
	}
	if t.dryRun {
		fmt.Fprintln(log, "\nDry run: nothing was written.")
		return "", nil
	}
	if !t.e.isRoot() {
		return "", errors.New("the installation must run as root")
	}
	unlock, err := lockInstall(t.lockPath)
	if err != nil {
		return "", err
	}
	defer unlock()
	if err := in.Run(ctx); err != nil {
		return "", err
	}
	return in.PasswordHash, nil
}

// quiet points the command echo of a real runner at log instead of stderr,
// which would scribble over the full-screen UI.
func quiet(r install.Runner, log io.Writer) install.Runner {
	if er, ok := r.(install.ExecRunner); ok {
		er.Log = log
		return er
	}
	return r
}

// lockInstall keeps two consoles (tty1 and ttyS0 run the installer at the
// same time) from installing at once; the text menu uses the same lock.
func lockInstall(path string) (func(), error) {
	if path == "" {
		return func() {}, nil
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return nil, err
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		f.Close()
		return nil, errors.New("an installation is already running on another console")
	}
	return func() { f.Close() }, nil
}

func (t *tuiEnv) SaveTargets(exclude string) []tui.SaveTarget {
	list, _ := disks.Probe(t.src, disks.Options{})
	return tui.FindSaveTargets(list, exclude, t.livePath)
}

func (t *tuiEnv) Save(ctx context.Context, st tui.SaveTarget, data []byte) (string, error) {
	if st.Device != "" && t.snapshot {
		return "", errors.New("a recorded snapshot has no real devices")
	}
	return tui.SaveToPartition(ctx, quiet(t.e.runner, nil), st, t.saveMnt, data)
}

func runTUI(args []string, stdin io.Reader, stdout, stderr io.Writer, e env) error {
	fs := flag.NewFlagSet("tui", flag.ContinueOnError)
	fs.SetOutput(stderr)
	dryRun := fs.Bool("dry-run", false, "go through every screen and print the plan; touch nothing")
	from := fs.String("from", "", "use a recorded disk snapshot (implies --dry-run)")
	answersPath := fs.String("answers", "", "pre-fill the screens from an answer file (repeat install)")
	lang := fs.String("lang", "", "preselect the language: uk, en or ru")
	langFile := fs.String("lang-file", "", "read the preselected language from FILE and write the chosen one back")
	actionFile := fs.String("action-file", "", "write the chosen final action (reboot, poweroff, menu) to FILE")
	logPath := fs.String("log", "/run/alpha-installer/install.log", "full installation log")
	lockPath := fs.String("lock", "/run/alpha-live/install.lock", "lock shared with the other consoles")
	livePath := fs.String("save-local", "/root/"+tui.AnswerFileName, "answer file offered in the live system")
	noSmart := fs.Bool("no-smart", false, "do not query SMART health")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() > 0 {
		return fmt.Errorf("unexpected argument %q", fs.Arg(0))
	}
	if !e.isTTY() {
		return errors.New("the installer TUI needs a terminal; use `install --answers FILE` otherwise")
	}

	t := &tuiEnv{e: e, src: e.live, resolve: e.resolve, dryRun: *dryRun, smart: !*noSmart, sysRoot: "/",
		lockPath: *lockPath, logPath: *logPath, livePath: *livePath, saveMnt: "/run/alpha-installer/save"}
	if *from != "" {
		t.src, t.resolve, t.dryRun, t.smart, t.snapshot = disks.Dir(*from), nil, true, false, true
	}
	if !e.isRoot() && !t.dryRun {
		return errors.New("the installation must run as root (or add --dry-run)")
	}
	if !e.isRoot() && *logPath == "/run/alpha-installer/install.log" {
		t.logPath = filepath.Join(os.TempDir(), "alpha-installer-install.log")
	}

	opt := tui.Options{Version: version, DryRun: t.dryRun, Lang: *lang, LogPath: t.logPath, Tick: time.Second}
	if *langFile != "" && opt.Lang == "" {
		if b, err := os.ReadFile(*langFile); err == nil {
			opt.Lang = string(bytes.TrimSpace(b))
		}
	}
	if *answersPath != "" {
		base, err := loadBase(*answersPath)
		if err != nil {
			return err
		}
		opt.Base = base
	}

	m := tui.New(t, opt)
	p := tea.NewProgram(m, tea.WithAltScreen(), tea.WithInput(stdin), tea.WithOutput(stdout))
	if _, err := p.Run(); err != nil {
		return fmt.Errorf("installer TUI: %w", err)
	}
	if *langFile != "" {
		_ = os.WriteFile(*langFile, []byte(m.Lang()+"\n"), 0o644)
	}
	if *actionFile != "" {
		return os.WriteFile(*actionFile, []byte(m.Action()+"\n"), 0o644)
	}
	if a := m.Action(); a != "" {
		fmt.Fprintln(stdout, a)
	}
	return nil
}

// loadBase reads an answer file for pre-filling: unknown fields are an
// error, missing ones are fine (the screens ask for them).
func loadBase(path string) (*answers.Answers, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	var a answers.Answers
	if err := dec.Decode(&a); err != nil {
		return nil, fmt.Errorf("answer file %s: %w", path, err)
	}
	return &a, nil
}
