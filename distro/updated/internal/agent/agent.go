// SPDX-License-Identifier: MPL-2.0

// Package agent is the core of alpha-updated: one operation at a time on top
// of bootc (check, download-only, apply, switch channel, rollback), persisted
// settings and state, and the maintenance-window scheduler that never reboots
// while the media server reports playback.
package agent

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/DenisHumen/kinolune/distro/updated/internal/bootc"
	"github.com/DenisHumen/kinolune/distro/updated/internal/settings"
)

// Bootc is the subset of bootc.Client the agent uses.
type Bootc interface {
	Status(ctx context.Context) (*bootc.Host, error)
	Check(ctx context.Context) error
	Download(ctx context.Context) error
	Unlock(ctx context.Context) error
	Upgrade(ctx context.Context) error
	Switch(ctx context.Context, image string) error
	Rollback(ctx context.Context) error
}

// Rebooter restarts the machine (systemctl reboot on a device).
type Rebooter interface {
	Reboot(ctx context.Context) error
}

// Errors the API maps to HTTP status codes.
var (
	ErrBusy       = errors.New("another operation is running")
	ErrNoUpdate   = errors.New("no update to apply")
	ErrNoRollback = errors.New("no previous deployment to roll back to")
	ErrInvalid    = errors.New("invalid request")
)

// Update phases (Status.Update.State).
const (
	PhaseUpToDate       = "up-to-date"
	PhaseAvailable      = "available"
	PhaseDownloaded     = "downloaded"
	PhaseStaged         = "staged"
	PhaseRollbackQueued = "rollback-queued"
	PhaseUnknown        = "unknown"
)

// Operation kinds.
const (
	OpCheck    = "check"
	OpDownload = "download"
	OpApply    = "apply"
	OpSwitch   = "switch"
	OpRollback = "rollback"
)

// Config tunes the agent.
type Config struct {
	// StateDir holds settings.json and state.json (/var/lib/alpha/updated).
	StateDir string
	// CheckInterval between automatic checks (default 6h).
	CheckInterval time.Duration
	// RetryAfter is the pause after a failed automatic operation (default 30m).
	RetryAfter time.Duration
	// DefaultRepo is used for a channel switch when the booted deployment has
	// no image reference (ALPHA_IMAGE_REPO from /usr/share/alpha/version).
	DefaultRepo string
	// OSVersion is shown in the status (ALPHA_OS_VERSION).
	OSVersion string
	Logger    *slog.Logger
	// Now is the clock (tests replace it).
	Now func() time.Time
}

// Operation is the running operation.
type Operation struct {
	Kind      string    `json:"kind"`
	StartedAt time.Time `json:"started_at"`
	// Auto is true when the scheduler started it.
	Auto bool `json:"auto"`
}

// OpError is the last failed operation.
type OpError struct {
	Op      string    `json:"op"`
	Message string    `json:"message"`
	At      time.Time `json:"at"`
}

// ApplyRecord remembers the last applied update across the reboot.
type ApplyRecord struct {
	Digest  string    `json:"digest"`
	Version string    `json:"version,omitempty"`
	At      time.Time `json:"at"`
	// Result: "pending" (rebooting), "booted" or "not-booted" (the old
	// deployment came back: greenboot rollback or a manual choice).
	Result string `json:"result"`
}

// state is persisted in state.json.
type state struct {
	LastCheck   time.Time `json:"last_check,omitzero"`
	LastAttempt time.Time `json:"last_attempt,omitzero"`
	// PendingApply: the user asked to apply in the next maintenance window.
	PendingApply bool `json:"pending_apply"`
	// Downloaded is the digest staged with --download-only and not yet
	// unlocked (for bootc versions that do not report downloadOnly).
	Downloaded string       `json:"downloaded,omitempty"`
	LastApply  *ApplyRecord `json:"last_apply,omitempty"`
	LastError  *OpError     `json:"last_error,omitempty"`
}

// Agent is safe for concurrent use.
type Agent struct {
	cfg    Config
	bootc  Bootc
	veto   Vetoer
	reboot Rebooter
	log    *slog.Logger
	store  settings.Store

	mu           sync.Mutex
	settings     settings.Settings
	haveSettings bool // settings.json exists
	st           state
	op           *Operation
	host         *bootc.Host
	hostErr      error
	failedAt     map[string]time.Time
	lastPostpone time.Time
	ctx          context.Context
	wg           sync.WaitGroup
}

// New loads settings and state from cfg.StateDir.
func New(cfg Config, b Bootc, v Vetoer, r Rebooter) *Agent {
	if cfg.CheckInterval <= 0 {
		cfg.CheckInterval = 6 * time.Hour
	}
	if cfg.RetryAfter <= 0 {
		cfg.RetryAfter = 30 * time.Minute
	}
	if cfg.Logger == nil {
		cfg.Logger = slog.New(slog.DiscardHandler)
	}
	if cfg.Now == nil {
		cfg.Now = time.Now
	}
	if v == nil {
		v = NoVeto{}
	}
	a := &Agent{
		cfg: cfg, bootc: b, veto: v, reboot: r, log: cfg.Logger,
		store:    settings.Store{Path: filepath.Join(cfg.StateDir, "settings.json")},
		failedAt: map[string]time.Time{},
		ctx:      context.Background(),
	}
	s, err := a.store.Load()
	if err != nil {
		a.log.Warn("settings unreadable, using defaults", "err", err)
	}
	a.settings = s
	if _, err := os.Stat(a.store.Path); err == nil {
		a.haveSettings = true
	}
	if data, err := os.ReadFile(a.statePath()); err == nil {
		if err := json.Unmarshal(data, &a.st); err != nil {
			a.log.Warn("state unreadable, starting fresh", "err", err)
			a.st = state{}
		}
	}
	return a
}

func (a *Agent) statePath() string { return filepath.Join(a.cfg.StateDir, "state.json") }

// saveState writes state.json; the caller holds a.mu.
func (a *Agent) saveState() {
	if err := settings.WriteJSONAtomic(a.statePath(), a.st); err != nil {
		a.log.Error("cannot save state", "err", err)
	}
}

// Start sets the context for background operations and reconciles the last
// applied update with what actually booted.
func (a *Agent) Start(ctx context.Context) {
	a.mu.Lock()
	a.ctx = ctx
	a.mu.Unlock()
	h := a.refresh(ctx)
	a.mu.Lock()
	defer a.mu.Unlock()
	if h == nil {
		return
	}
	booted := h.Status.Booted.Digest()
	if !a.haveSettings && h.Spec.Image != nil {
		// A fresh device follows the channel it was installed from.
		if tag := bootc.Tag(h.Spec.Image.Image); validChannel(tag) {
			a.settings.Channel = tag
		}
	}
	if la := a.st.LastApply; la != nil && la.Result == "pending" {
		if booted == la.Digest {
			la.Result = "booted"
			a.log.Info("update booted", "digest", la.Digest, "version", la.Version)
		} else {
			la.Result = "not-booted"
			a.log.Warn("applied update did not boot, running the previous deployment",
				"wanted", la.Digest, "booted", booted)
		}
		a.saveState()
	}
	if a.st.Downloaded != "" && h.Status.Staged.Digest() != a.st.Downloaded {
		a.st.Downloaded = "" // the staged download is gone (applied or discarded)
		a.saveState()
	}
}

func validChannel(c string) bool {
	for _, x := range settings.Channels {
		if c == x {
			return true
		}
	}
	return false
}

// Wait blocks until background operations finish (tests, shutdown).
func (a *Agent) Wait() { a.wg.Wait() }

// refresh reads bootc status and caches it; returns nil on error.
func (a *Agent) refresh(ctx context.Context) *bootc.Host {
	h, err := a.bootc.Status(ctx)
	a.mu.Lock()
	defer a.mu.Unlock()
	if err != nil {
		a.hostErr = err
		a.log.Error("bootc status failed", "err", err)
		return nil
	}
	a.host, a.hostErr = h, nil
	return h
}

// phase computes the update phase; caller holds a.mu.
func (a *Agent) phase(h *bootc.Host) string {
	if h == nil {
		return PhaseUnknown
	}
	if h.Status.RollbackQueued {
		return PhaseRollbackQueued
	}
	if s := h.Status.Staged; s != nil {
		if s.DownloadOnly || (a.st.Downloaded != "" && a.st.Downloaded == s.Digest()) {
			return PhaseDownloaded
		}
		return PhaseStaged
	}
	if h.Available() != nil {
		return PhaseAvailable
	}
	return PhaseUpToDate
}

// begin marks kind as running; false if something else runs. Holds a.mu.
func (a *Agent) begin(kind string, auto bool) bool {
	if a.op != nil {
		return false
	}
	a.op = &Operation{Kind: kind, StartedAt: a.cfg.Now(), Auto: auto}
	return true
}

// finish clears the running operation and records the result.
func (a *Agent) finish(kind string, auto bool, started time.Time, err error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.op = nil
	dur := a.cfg.Now().Sub(started)
	if err != nil {
		a.st.LastError = &OpError{Op: kind, Message: err.Error(), At: a.cfg.Now()}
		if auto {
			a.failedAt[kind] = a.cfg.Now()
		}
		a.log.Error("operation failed", "op", kind, "auto", auto, "duration", dur, "err", err)
	} else {
		if a.st.LastError != nil && a.st.LastError.Op == kind {
			a.st.LastError = nil
		}
		delete(a.failedAt, kind)
		a.log.Info("operation done", "op", kind, "auto", auto, "duration", dur)
	}
	a.saveState()
}

// run executes fn as operation kind, synchronously.
func (a *Agent) run(ctx context.Context, kind string, auto bool, fn func(context.Context) error) error {
	a.mu.Lock()
	if !a.begin(kind, auto) {
		a.mu.Unlock()
		return ErrBusy
	}
	started := a.op.StartedAt
	a.mu.Unlock()
	a.log.Info("operation started", "op", kind, "auto", auto)
	err := fn(ctx)
	a.refresh(ctx)
	a.finish(kind, auto, started, err)
	return err
}

// async starts fn as operation kind in the background.
func (a *Agent) async(kind string, fn func(context.Context) error) (Operation, error) {
	a.mu.Lock()
	if !a.begin(kind, false) {
		op := *a.op
		a.mu.Unlock()
		return op, ErrBusy
	}
	op := *a.op
	ctx := a.ctx
	a.wg.Add(1)
	a.mu.Unlock()
	a.log.Info("operation started", "op", kind, "auto", false)
	go func() {
		defer a.wg.Done()
		err := fn(ctx)
		a.refresh(ctx)
		a.finish(kind, false, op.StartedAt, err)
	}()
	return op, nil
}

// --- operations -------------------------------------------------------

func (a *Agent) doCheck(ctx context.Context) error {
	a.mu.Lock()
	a.st.LastAttempt = a.cfg.Now()
	a.mu.Unlock()
	if err := a.bootc.Check(ctx); err != nil {
		return err
	}
	a.mu.Lock()
	a.st.LastCheck = a.cfg.Now()
	a.mu.Unlock()
	if h, err := a.bootc.Status(ctx); err == nil {
		if av := h.Available(); av != nil {
			a.log.Info("update available", "version", av.Version, "digest", av.ImageDigest)
		} else {
			a.log.Info("no update available")
		}
	}
	return nil
}

func (a *Agent) doDownload(ctx context.Context) error {
	h, err := a.bootc.Status(ctx)
	if err != nil {
		return err
	}
	if h.Status.Staged != nil {
		return nil // already downloaded or staged
	}
	if h.Available() == nil {
		if err := a.doCheck(ctx); err != nil {
			return err
		}
		if h, err = a.bootc.Status(ctx); err != nil {
			return err
		}
		if h.Available() == nil {
			return ErrNoUpdate
		}
	}
	if err := a.bootc.Download(ctx); err != nil {
		return err
	}
	h, err = a.bootc.Status(ctx)
	if err != nil {
		return err
	}
	a.mu.Lock()
	a.st.Downloaded = h.Status.Staged.Digest()
	a.mu.Unlock()
	a.log.Info("update downloaded", "digest", h.Status.Staged.Digest())
	return nil
}

// doApply makes the update the next boot and reboots.
func (a *Agent) doApply(ctx context.Context, auto bool) error {
	h, err := a.bootc.Status(ctx)
	if err != nil {
		return err
	}
	a.mu.Lock()
	ph := a.phase(h)
	a.mu.Unlock()
	switch ph {
	case PhaseDownloaded:
		if err := a.bootc.Unlock(ctx); err != nil {
			return err
		}
		a.mu.Lock()
		a.st.Downloaded = "" // unlocked: from now on a normal staged deployment
		a.mu.Unlock()
	case PhaseStaged:
		// nothing to do: the staged deployment boots next
	case PhaseAvailable:
		if err := a.bootc.Upgrade(ctx); err != nil {
			return err
		}
	case PhaseUpToDate:
		if err := a.doCheck(ctx); err != nil {
			return err
		}
		if h, err = a.bootc.Status(ctx); err != nil {
			return err
		}
		if h.Available() == nil {
			return ErrNoUpdate
		}
		if err := a.bootc.Upgrade(ctx); err != nil {
			return err
		}
	default:
		return fmt.Errorf("%w: update state is %s", ErrInvalid, ph)
	}
	h, err = a.bootc.Status(ctx)
	if err != nil {
		return err
	}
	if h.Status.Staged == nil {
		return errors.New("bootc did not stage a deployment")
	}
	if auto {
		// Last look right before the reboot: playback may have started while
		// we were unlocking the deployment.
		if busy, reason := a.vetoed(ctx); busy {
			a.log.Info("reboot postponed: media server is busy", "reason", reason)
			return nil // staged and unlocked; the next tick in the window reboots
		}
	}
	rec := &ApplyRecord{Digest: h.Status.Staged.Digest(), At: a.cfg.Now(), Result: "pending"}
	if h.Status.Staged.Image != nil {
		rec.Version = h.Status.Staged.Image.Version
	}
	a.mu.Lock()
	a.st.PendingApply = false
	a.st.Downloaded = ""
	a.st.LastApply = rec
	a.saveState()
	a.mu.Unlock()
	a.log.Info("rebooting into the update", "digest", rec.Digest, "version", rec.Version, "auto", auto)
	return a.reboot.Reboot(ctx)
}

// --- public API ---------------------------------------------------------

// Check starts `bootc upgrade --check` in the background.
func (a *Agent) Check() (Operation, error) {
	return a.async(OpCheck, a.doCheck)
}

// Download starts `bootc upgrade --download-only` in the background.
func (a *Agent) Download() (Operation, error) {
	return a.async(OpDownload, a.doDownload)
}

// Apply: when "now" applies and reboots at once (an explicit user action is
// not vetoed); when "maintenance" marks the update for the next window.
func (a *Agent) Apply(when string) (*Operation, error) {
	switch when {
	case "", "now":
		op, err := a.async(OpApply, func(ctx context.Context) error { return a.doApply(ctx, false) })
		return &op, err
	case "maintenance":
		a.mu.Lock()
		a.st.PendingApply = true
		a.saveState()
		a.mu.Unlock()
		a.log.Info("update will be applied in the maintenance window")
		return nil, nil
	default:
		return nil, fmt.Errorf("%w: when must be \"now\" or \"maintenance\"", ErrInvalid)
	}
}

// CancelPending drops a "maintenance" apply request.
func (a *Agent) CancelPending() {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.st.PendingApply = false
	a.saveState()
}

// Rollback queues the previous deployment for the next boot and, if reboot
// is true, reboots into it.
func (a *Agent) Rollback(reboot bool) (Operation, error) {
	return a.async(OpRollback, func(ctx context.Context) error {
		h, err := a.bootc.Status(ctx)
		if err != nil {
			return err
		}
		if !h.Status.RollbackQueued {
			if h.Status.Rollback == nil {
				return ErrNoRollback
			}
			if err := a.bootc.Rollback(ctx); err != nil {
				return err
			}
			a.log.Info("rollback queued", "to", h.Status.Rollback.Digest())
		}
		a.mu.Lock()
		a.st.PendingApply = false
		a.st.Downloaded = ""
		a.saveState()
		a.mu.Unlock()
		if reboot {
			a.log.Info("rebooting into the previous deployment")
			return a.reboot.Reboot(ctx)
		}
		return nil
	})
}

// Settings returns the current settings.
func (a *Agent) Settings() settings.Settings {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.settings
}

// SetSettings validates and stores a patch. A channel change starts
// `bootc switch <repo>:<channel>` in the background (returned operation).
func (a *Agent) SetSettings(p settings.Patch) (settings.Settings, *Operation, error) {
	a.mu.Lock()
	next := a.settings.Apply(p)
	if err := next.Validate(); err != nil {
		a.mu.Unlock()
		return a.settings, nil, fmt.Errorf("%w: %v", ErrInvalid, err)
	}
	var target string
	if h := a.host; h != nil && p.Channel != nil {
		repo := a.cfg.DefaultRepo
		cur := ""
		if h.Spec.Image != nil && h.Spec.Image.Image != "" {
			repo = bootc.Repository(h.Spec.Image.Image)
			cur = h.Spec.Image.Image
		}
		if want := repo + ":" + next.Channel; repo != "" && want != cur {
			target = want
		}
	}
	if target != "" && a.op != nil {
		a.mu.Unlock()
		return a.settings, nil, ErrBusy
	}
	prev := a.settings
	if err := a.store.Save(next); err != nil {
		a.mu.Unlock()
		return prev, nil, err
	}
	a.settings, a.haveSettings = next, true
	a.mu.Unlock()
	a.log.Info("settings changed", "channel", next.Channel, "auto", next.Auto, "window", next.Window)
	if target == "" {
		return next, nil, nil
	}
	op, err := a.async(OpSwitch, func(ctx context.Context) error {
		a.log.Info("switching channel", "image", target)
		if err := a.bootc.Switch(ctx, target); err != nil {
			return err
		}
		a.mu.Lock()
		a.st.Downloaded = ""
		a.mu.Unlock()
		return nil
	})
	if err != nil {
		return next, nil, err
	}
	return next, &op, nil
}

// --- scheduler ------------------------------------------------------------

// Tick runs one scheduler step at time now: periodic check, background
// download, apply in the maintenance window. Operations run synchronously;
// call it from one goroutine (Run).
func (a *Agent) Tick(ctx context.Context, now time.Time) {
	a.mu.Lock()
	if a.op != nil {
		a.mu.Unlock()
		return
	}
	s, st := a.settings, a.st
	a.mu.Unlock()

	retry := func(kind string) bool {
		a.mu.Lock()
		defer a.mu.Unlock()
		at, failed := a.failedAt[kind]
		return !failed || now.Sub(at) >= a.cfg.RetryAfter
	}

	if now.Sub(st.LastCheck) >= a.cfg.CheckInterval && now.Sub(st.LastAttempt) >= a.cfg.RetryAfter {
		_ = a.run(ctx, OpCheck, true, a.doCheck)
	}

	h := a.refresh(ctx)
	if h == nil {
		return
	}
	a.mu.Lock()
	ph := a.phase(h)
	pending := a.st.PendingApply
	a.mu.Unlock()

	if ph == PhaseAvailable && (s.Auto != settings.AutoOff || pending) && retry(OpDownload) {
		if err := a.run(ctx, OpDownload, true, a.doDownload); err != nil {
			return
		}
		if h = a.refresh(ctx); h == nil {
			return
		}
		a.mu.Lock()
		ph = a.phase(h)
		a.mu.Unlock()
	}

	if !(s.Auto == settings.AutoInstall || pending) {
		return
	}
	if ph != PhaseDownloaded && ph != PhaseStaged {
		return
	}
	w, err := settings.ParseWindow(s.Window)
	if err != nil || !w.Contains(now) {
		return
	}
	if busy, reason := a.vetoed(ctx); busy {
		a.mu.Lock()
		logIt := now.Sub(a.lastPostpone) >= 15*time.Minute
		if logIt {
			a.lastPostpone = now
		}
		a.mu.Unlock()
		if logIt {
			a.log.Info("update postponed: media server is busy", "reason", reason)
		}
		return
	}
	if !retry(OpApply) {
		return
	}
	_ = a.run(ctx, OpApply, true, func(ctx context.Context) error { return a.doApply(ctx, true) })
}

// vetoed asks the media server; an error counts as busy (never reboot blind).
func (a *Agent) vetoed(ctx context.Context) (bool, string) {
	busy, reason, err := a.veto.Busy(ctx)
	if err != nil {
		a.log.Warn("cannot ask the media server about playback, postponing", "err", err)
		return true, "media server did not answer: " + err.Error()
	}
	return busy, reason
}

// Run calls Tick every interval until ctx ends, after an initial delay.
func (a *Agent) Run(ctx context.Context, delay, every time.Duration) {
	t := time.NewTimer(delay)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			a.Tick(ctx, a.cfg.Now())
			t.Reset(every)
		}
	}
}

// --- status ---------------------------------------------------------------

// Deployment is the API view of a bootc deployment.
type Deployment struct {
	Image        string `json:"image,omitempty"`
	Version      string `json:"version,omitempty"`
	Digest       string `json:"digest"`
	Timestamp    string `json:"timestamp,omitempty"`
	DownloadOnly bool   `json:"download_only,omitempty"`
	Pinned       bool   `json:"pinned,omitempty"`
}

func deployment(e *bootc.BootEntry) *Deployment {
	if e == nil || e.Image == nil {
		return nil
	}
	return &Deployment{
		Image: e.Image.Image.Image, Version: e.Image.Version, Digest: e.Image.ImageDigest,
		Timestamp: e.Image.Timestamp, DownloadOnly: e.DownloadOnly, Pinned: e.Pinned,
	}
}

// UpdateInfo is GET /v1/update.
type UpdateInfo struct {
	State          string      `json:"state"`
	CurrentVersion string      `json:"current_version,omitempty"`
	Available      *Deployment `json:"available,omitempty"`
	PendingApply   bool        `json:"pending_apply"`
	LastCheck      *time.Time  `json:"last_check,omitempty"`
	Operation      *Operation  `json:"operation"`
	LastError      *OpError    `json:"last_error,omitempty"`
}

// Status is GET /v1/status.
type Status struct {
	OSVersion      string            `json:"os_version,omitempty"`
	Tracking       string            `json:"tracking,omitempty"`
	Booted         *Deployment       `json:"booted,omitempty"`
	Staged         *Deployment       `json:"staged,omitempty"`
	Rollback       *Deployment       `json:"rollback,omitempty"`
	RollbackQueued bool              `json:"rollback_queued"`
	Update         UpdateInfo        `json:"update"`
	LastApply      *ApplyRecord      `json:"last_apply,omitempty"`
	Settings       settings.Settings `json:"settings"`
	NextWindow     *time.Time        `json:"next_window,omitempty"`
	BootcError     string            `json:"bootc_error,omitempty"`
}

// Status refreshes bootc status unless an operation runs (then the cached
// one is shown, bootc may hold its lock).
func (a *Agent) Status(ctx context.Context) Status {
	a.mu.Lock()
	running := a.op != nil
	a.mu.Unlock()
	if !running {
		a.refresh(ctx)
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	h := a.host
	s := Status{OSVersion: a.cfg.OSVersion, Settings: a.settings, LastApply: a.st.LastApply}
	s.Update = UpdateInfo{State: a.phase(h), PendingApply: a.st.PendingApply, LastError: a.st.LastError}
	if a.op != nil {
		op := *a.op
		s.Update.Operation = &op
	}
	if !a.st.LastCheck.IsZero() {
		t := a.st.LastCheck
		s.Update.LastCheck = &t
	}
	if a.hostErr != nil {
		s.BootcError = a.hostErr.Error()
	}
	if h != nil {
		if h.Spec.Image != nil {
			s.Tracking = h.Spec.Image.Image
		}
		s.Booted = deployment(h.Status.Booted)
		s.Staged = deployment(h.Status.Staged)
		s.Rollback = deployment(h.Status.Rollback)
		s.RollbackQueued = h.Status.RollbackQueued
		if s.Booted != nil {
			s.Update.CurrentVersion = s.Booted.Version
		}
		switch {
		case s.Staged != nil:
			s.Update.Available = s.Staged
		case h.Available() != nil:
			av := h.Available()
			s.Update.Available = &Deployment{Image: av.Image.Image, Version: av.Version, Digest: av.ImageDigest, Timestamp: av.Timestamp}
		}
	}
	if w, err := settings.ParseWindow(a.settings.Window); err == nil &&
		(a.settings.Auto == settings.AutoInstall || a.st.PendingApply) {
		t := w.NextStart(a.cfg.Now())
		s.NextWindow = &t
	}
	return s
}
