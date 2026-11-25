// SPDX-License-Identifier: MPL-2.0

package agent

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/DenisHumen/kinolune/distro/updated/internal/bootc"
	"github.com/DenisHumen/kinolune/distro/updated/internal/fakebootc"
	"github.com/DenisHumen/kinolune/distro/updated/internal/settings"
)

const ref = "ghcr.io/denishumen/alpha-os:stable"

var (
	v1 = fakebootc.Image{Digest: "sha256:v1", Version: "45.20261101.1"}
	v2 = fakebootc.Image{Digest: "sha256:v2", Version: "45.20261120.1"}
	v3 = fakebootc.Image{Digest: "sha256:v3", Version: "45.20261122.1"}
)

type clock struct {
	mu sync.Mutex
	t  time.Time
}

func (c *clock) Now() time.Time  { c.mu.Lock(); defer c.mu.Unlock(); return c.t }
func (c *clock) Set(t time.Time) { c.mu.Lock(); c.t = t; c.mu.Unlock() }

// day returns 2026-11-25 hh:mm in UTC.
func day(h, m int) time.Time { return time.Date(2026, 11, 25, h, m, 0, 0, time.UTC) }

// fakeVeto answers from a queue; the last answer repeats.
type fakeVeto struct {
	mu      sync.Mutex
	answers []vetoAnswer
	calls   int
}

type vetoAnswer struct {
	busy bool
	err  error
}

func (v *fakeVeto) set(a ...vetoAnswer) { v.mu.Lock(); v.answers = a; v.mu.Unlock() }

func (v *fakeVeto) Busy(context.Context) (bool, string, error) {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.calls++
	if len(v.answers) == 0 {
		return false, "", nil
	}
	a := v.answers[0]
	if len(v.answers) > 1 {
		v.answers = v.answers[1:]
	}
	return a.busy, "someone is watching", a.err
}

type env struct {
	t       *testing.T
	dir     string
	fb      *fakebootc.Fake
	clk     *clock
	veto    *fakeVeto
	reboots atomic.Int32
	// onReboot simulates what happens on reboot (default: fb.Boot).
	onReboot func()
	a        *Agent
}

func newEnv(t *testing.T, bootRef string) *env {
	t.Helper()
	e := &env{t: t, dir: t.TempDir(), fb: fakebootc.New(bootRef, v1), clk: &clock{t: day(14, 0)}, veto: &fakeVeto{}}
	e.onReboot = e.fb.Boot
	e.restart()
	return e
}

// restart builds a new agent on the same state dir (daemon restart or boot).
func (e *env) restart() {
	e.a = New(Config{
		StateDir:    e.dir,
		DefaultRepo: "ghcr.io/denishumen/alpha-os",
		Now:         e.clk.Now,
	}, bootc.Client{R: e.fb}, e.veto, RebootFunc(func(context.Context) error {
		e.reboots.Add(1)
		e.onReboot()
		return nil
	}))
	e.a.Start(context.Background())
}

func (e *env) state() string { return e.a.Status(context.Background()).Update.State }

func (e *env) called(cmd string) bool { return slices.Contains(e.fb.CallList(), cmd) }

func (e *env) count(cmd string) int {
	n := 0
	for _, c := range e.fb.CallList() {
		if c == cmd {
			n++
		}
	}
	return n
}

func (e *env) settings(p settings.Patch) {
	e.t.Helper()
	if _, _, err := e.a.SetSettings(p); err != nil {
		e.t.Fatal(err)
	}
	e.a.Wait()
}

func ptr[T any](v T) *T { return &v }

func TestCheckFindsUpdate(t *testing.T) {
	e := newEnv(t, ref)
	if got := e.state(); got != PhaseUpToDate {
		t.Fatalf("state = %s", got)
	}
	e.fb.Publish(ref, v2)
	if _, err := e.a.Check(); err != nil {
		t.Fatal(err)
	}
	e.a.Wait()
	st := e.a.Status(context.Background())
	if st.Update.State != PhaseAvailable || st.Update.Available == nil || st.Update.Available.Version != v2.Version {
		t.Fatalf("update = %+v", st.Update)
	}
	if st.Update.LastCheck == nil || !st.Update.LastCheck.Equal(day(14, 0)) {
		t.Errorf("last check = %v", st.Update.LastCheck)
	}
	if st.Booted == nil || st.Booted.Version != v1.Version || st.Tracking != ref {
		t.Errorf("booted = %+v tracking %q", st.Booted, st.Tracking)
	}
}

func TestDownloadOnlyThenApplyNow(t *testing.T) {
	e := newEnv(t, ref)
	e.fb.Publish(ref, v2)
	if _, err := e.a.Download(); err != nil {
		t.Fatal(err)
	}
	e.a.Wait()
	st := e.a.Status(context.Background())
	if st.Update.State != PhaseDownloaded || st.Staged == nil || !st.Staged.DownloadOnly {
		t.Fatalf("after download: %+v staged %+v", st.Update, st.Staged)
	}
	if !e.called("upgrade --download-only") || e.reboots.Load() != 0 {
		t.Fatalf("calls %v reboots %d", e.fb.CallList(), e.reboots.Load())
	}
	// A reboot for another reason does not apply a download-only update.
	e.fb.Boot()
	if e.fb.Booted() != v1.Digest {
		t.Fatal("download-only deployment was applied by a plain reboot")
	}

	if _, err := e.a.Apply("now"); err != nil {
		t.Fatal(err)
	}
	e.a.Wait()
	if !e.called("upgrade --from-downloaded") || e.reboots.Load() != 1 {
		t.Fatalf("calls %v reboots %d", e.fb.CallList(), e.reboots.Load())
	}
	if e.fb.Booted() != v2.Digest {
		t.Fatalf("booted %s", e.fb.Booted())
	}
	e.restart()
	st = e.a.Status(context.Background())
	if st.LastApply == nil || st.LastApply.Result != "booted" || st.LastApply.Version != v2.Version {
		t.Fatalf("last apply = %+v", st.LastApply)
	}
	if st.Update.State != PhaseUpToDate || st.Rollback == nil || st.Rollback.Digest != v1.Digest {
		t.Fatalf("after boot: %+v rollback %+v", st.Update, st.Rollback)
	}
}

func TestOldBootcWithoutDownloadOnlyField(t *testing.T) {
	e := newEnv(t, ref)
	e.fb.OldBootc = true
	e.fb.Publish(ref, v2)
	if _, err := e.a.Download(); err != nil {
		t.Fatal(err)
	}
	e.a.Wait()
	if got := e.state(); got != PhaseDownloaded {
		t.Fatalf("state = %s", got)
	}
	e.restart() // the marker survives a daemon restart
	if got := e.state(); got != PhaseDownloaded {
		t.Fatalf("after restart state = %s", got)
	}
	if _, err := e.a.Apply("now"); err != nil {
		t.Fatal(err)
	}
	e.a.Wait()
	if e.fb.Booted() != v2.Digest {
		t.Fatalf("booted %s, calls %v", e.fb.Booted(), e.fb.CallList())
	}
}

func TestApplyNowFromAvailable(t *testing.T) {
	e := newEnv(t, ref)
	e.fb.Publish(ref, v2)
	if _, err := e.a.Apply("now"); err != nil {
		t.Fatal(err)
	}
	e.a.Wait()
	if !e.called("upgrade --check") || !e.called("upgrade") || e.called("upgrade --from-downloaded") {
		t.Fatalf("calls %v", e.fb.CallList())
	}
	if e.reboots.Load() != 1 || e.fb.Booted() != v2.Digest {
		t.Fatalf("reboots %d booted %s", e.reboots.Load(), e.fb.Booted())
	}
}

func TestApplyNothing(t *testing.T) {
	e := newEnv(t, ref)
	if _, err := e.a.Apply("now"); err != nil {
		t.Fatal(err)
	}
	e.a.Wait()
	st := e.a.Status(context.Background())
	if st.Update.LastError == nil || st.Update.LastError.Op != OpApply || !strings.Contains(st.Update.LastError.Message, ErrNoUpdate.Error()) {
		t.Fatalf("last error = %+v", st.Update.LastError)
	}
	if e.reboots.Load() != 0 {
		t.Fatal("rebooted without an update")
	}
	if _, err := e.a.Apply("tomorrow"); !errors.Is(err, ErrInvalid) {
		t.Fatalf("bad when: %v", err)
	}
}

func TestBootcErrorsAreReported(t *testing.T) {
	e := newEnv(t, ref)
	e.fb.Publish(ref, v2)
	e.fb.SetFail("upgrade --download-only", "error: writing objects: No space left on device")
	if _, err := e.a.Download(); err != nil {
		t.Fatal(err)
	}
	e.a.Wait()
	st := e.a.Status(context.Background())
	if st.Update.LastError == nil || !strings.Contains(st.Update.LastError.Message, "No space left") {
		t.Fatalf("last error = %+v", st.Update.LastError)
	}
	if st.Update.State != PhaseAvailable {
		t.Fatalf("state = %s", st.Update.State)
	}
	// Success of the same operation clears the error.
	e.fb.SetFail("upgrade --download-only", "")
	e.a.Download()
	e.a.Wait()
	if st := e.a.Status(context.Background()); st.Update.LastError != nil {
		t.Fatalf("error not cleared: %+v", st.Update.LastError)
	}

	e.fb.SetFail("status --format=json", "error: Opening sysroot: permission denied")
	if st := e.a.Status(context.Background()); !strings.Contains(st.BootcError, "permission denied") {
		t.Fatalf("bootc error = %q", st.BootcError)
	}
}

func TestRollback(t *testing.T) {
	e := newEnv(t, ref)
	if _, err := e.a.Rollback(true); err != nil {
		t.Fatal(err)
	}
	e.a.Wait()
	if st := e.a.Status(context.Background()); st.Update.LastError == nil || !strings.Contains(st.Update.LastError.Message, ErrNoRollback.Error()) {
		t.Fatalf("rollback without previous: %+v", st.Update.LastError)
	}

	e.fb.Publish(ref, v2)
	e.a.Apply("now")
	e.a.Wait()
	e.restart()
	if e.fb.Booted() != v2.Digest {
		t.Fatal("setup: v2 not booted")
	}
	// Queue only: the user reboots later.
	if _, err := e.a.Rollback(false); err != nil {
		t.Fatal(err)
	}
	e.a.Wait()
	if got := e.state(); got != PhaseRollbackQueued || e.reboots.Load() != 1 {
		t.Fatalf("state %s reboots %d", got, e.reboots.Load())
	}
	// Rollback again with reboot: already queued, so only the reboot happens.
	e.a.Rollback(true)
	e.a.Wait()
	if e.count("rollback") != 1 || e.reboots.Load() != 2 || e.fb.Booted() != v1.Digest {
		t.Fatalf("calls %v reboots %d booted %s", e.fb.CallList(), e.reboots.Load(), e.fb.Booted())
	}
}

func TestOneOperationAtATime(t *testing.T) {
	e := newEnv(t, ref)
	e.fb.Registry["ghcr.io/denishumen/alpha-os:beta"] = v3
	block := make(chan struct{})
	e.fb.Block = block
	if _, err := e.a.Check(); err != nil {
		t.Fatal(err)
	}
	if op, err := e.a.Download(); !errors.Is(err, ErrBusy) || op.Kind != OpCheck {
		t.Fatalf("download while checking: %+v %v", op, err)
	}
	if _, _, err := e.a.SetSettings(settings.Patch{Channel: ptr("beta")}); !errors.Is(err, ErrBusy) {
		t.Fatalf("channel switch while checking: %v", err)
	}
	if got := e.a.Settings().Channel; got != "stable" {
		t.Fatalf("channel changed although busy: %s", got)
	}
	// Status does not call bootc during an operation and shows it.
	if st := e.a.Status(context.Background()); st.Update.Operation == nil || st.Update.Operation.Kind != OpCheck {
		t.Fatalf("operation = %+v", st.Update.Operation)
	}
	// Settings without a channel change are fine while busy.
	if _, _, err := e.a.SetSettings(settings.Patch{Window: ptr("02:00-04:00")}); err != nil {
		t.Fatal(err)
	}
	close(block)
	e.a.Wait()
	e.fb.Block = nil
	if st := e.a.Status(context.Background()); st.Update.Operation != nil {
		t.Fatalf("operation left: %+v", st.Update.Operation)
	}
}

func TestSettingsPersistAndChannelSwitch(t *testing.T) {
	e := newEnv(t, ref)
	e.fb.Registry["ghcr.io/denishumen/alpha-os:beta"] = v3
	if _, _, err := e.a.SetSettings(settings.Patch{Channel: ptr("nightly")}); !errors.Is(err, ErrInvalid) {
		t.Fatalf("invalid channel: %v", err)
	}
	if _, err := os.Stat(filepath.Join(e.dir, "settings.json")); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("invalid settings were written")
	}
	_, op, err := e.a.SetSettings(settings.Patch{Channel: ptr("beta"), Auto: ptr(settings.AutoDownload), Window: ptr("01:00-02:30")})
	if err != nil || op == nil || op.Kind != OpSwitch {
		t.Fatalf("switch: %+v %v", op, err)
	}
	e.a.Wait()
	if !e.called("switch ghcr.io/denishumen/alpha-os:beta") {
		t.Fatalf("calls %v", e.fb.CallList())
	}
	st := e.a.Status(context.Background())
	if st.Update.State != PhaseStaged || st.Tracking != "ghcr.io/denishumen/alpha-os:beta" {
		t.Fatalf("after switch: %s tracking %s", st.Update.State, st.Tracking)
	}
	e.restart()
	want := settings.Settings{Channel: "beta", Auto: settings.AutoDownload, Window: "01:00-02:30"}
	if got := e.a.Settings(); got != want {
		t.Fatalf("reloaded settings = %+v", got)
	}
	// Same channel again: no second switch.
	e.settings(settings.Patch{Channel: ptr("beta")})
	if e.count("switch ghcr.io/denishumen/alpha-os:beta") != 1 {
		t.Fatalf("calls %v", e.fb.CallList())
	}
}

func TestFreshDeviceFollowsInstalledChannel(t *testing.T) {
	e := newEnv(t, "ghcr.io/denishumen/alpha-os:beta")
	if got := e.a.Settings().Channel; got != "beta" {
		t.Fatalf("channel = %s", got)
	}
	e2 := newEnv(t, "localhost:5000/alpha-os:dev")
	if got := e2.a.Settings().Channel; got != "stable" {
		t.Fatalf("dev image channel = %s", got)
	}
}

func TestNotBootedAfterGreenbootRollback(t *testing.T) {
	e := newEnv(t, ref)
	e.onReboot = e.fb.GreenbootRollback
	e.fb.Publish(ref, v2)
	e.a.Apply("now")
	e.a.Wait()
	e.restart()
	st := e.a.Status(context.Background())
	if st.LastApply == nil || st.LastApply.Result != "not-booted" || st.LastApply.Digest != v2.Digest {
		t.Fatalf("last apply = %+v", st.LastApply)
	}
}

// --- scheduler ---

func TestSchedulerDownloadsThenAppliesInWindow(t *testing.T) {
	e := newEnv(t, ref)
	e.fb.Publish(ref, v2)
	ctx := context.Background()

	e.a.Tick(ctx, e.clk.Now()) // 14:00: check + download, no reboot
	if !e.called("upgrade --check") || !e.called("upgrade --download-only") || e.reboots.Load() != 0 {
		t.Fatalf("calls %v reboots %d", e.fb.CallList(), e.reboots.Load())
	}
	if got := e.state(); got != PhaseDownloaded {
		t.Fatalf("state = %s", got)
	}
	st := e.a.Status(ctx)
	if st.NextWindow == nil || !st.NextWindow.Equal(day(3, 0).AddDate(0, 0, 1)) {
		t.Fatalf("next window = %v", st.NextWindow)
	}
	e.clk.Set(day(18, 0))
	e.a.Tick(ctx, e.clk.Now())
	if e.reboots.Load() != 0 {
		t.Fatal("rebooted outside the window")
	}
	if e.count("upgrade --check") != 1 {
		t.Fatalf("checked again before the interval: %v", e.fb.CallList())
	}
	e.clk.Set(day(3, 10).AddDate(0, 0, 1))
	e.a.Tick(ctx, e.clk.Now())
	if e.reboots.Load() != 1 || e.fb.Booted() != v2.Digest {
		t.Fatalf("reboots %d booted %s calls %v", e.reboots.Load(), e.fb.Booted(), e.fb.CallList())
	}
}

func TestSchedulerNeverRebootsDuringPlayback(t *testing.T) {
	e := newEnv(t, ref)
	e.fb.Publish(ref, v2)
	ctx := context.Background()
	e.clk.Set(day(3, 0))
	e.veto.set(vetoAnswer{busy: true})
	for m := 0; m < 120; m += 5 { // the whole window, someone watches
		e.clk.Set(day(3, m))
		e.a.Tick(ctx, e.clk.Now())
	}
	if e.reboots.Load() != 0 || e.called("upgrade --from-downloaded") {
		t.Fatalf("rebooted during playback: reboots %d calls %v", e.reboots.Load(), e.fb.CallList())
	}
	if got := e.state(); got != PhaseDownloaded {
		t.Fatalf("state = %s", got)
	}
	// The media server cannot answer: still no reboot.
	e.veto.set(vetoAnswer{err: errors.New("timeout")})
	e.clk.Set(day(3, 0).AddDate(0, 0, 1))
	e.a.Tick(ctx, e.clk.Now())
	if e.reboots.Load() != 0 {
		t.Fatal("rebooted although the media server did not answer")
	}
	// Playback ended: the update goes in.
	e.veto.set(vetoAnswer{})
	e.clk.Set(day(3, 30).AddDate(0, 0, 1))
	e.a.Tick(ctx, e.clk.Now())
	if e.reboots.Load() != 1 {
		t.Fatal("no reboot after playback ended")
	}
}

func TestPlaybackStartsWhileApplying(t *testing.T) {
	e := newEnv(t, ref)
	e.fb.Publish(ref, v2)
	ctx := context.Background()
	e.a.Tick(ctx, e.clk.Now()) // download at 14:00
	// First answer (before unlock) free, second (right before reboot) busy.
	e.veto.set(vetoAnswer{}, vetoAnswer{busy: true})
	e.clk.Set(day(3, 0).AddDate(0, 0, 1))
	e.a.Tick(ctx, e.clk.Now())
	if e.reboots.Load() != 0 || !e.called("upgrade --from-downloaded") {
		t.Fatalf("reboots %d calls %v", e.reboots.Load(), e.fb.CallList())
	}
	if got := e.state(); got != PhaseStaged {
		t.Fatalf("state = %s", got)
	}
	e.veto.set(vetoAnswer{})
	e.clk.Set(day(3, 5).AddDate(0, 0, 1))
	e.a.Tick(ctx, e.clk.Now())
	if e.reboots.Load() != 1 || e.count("upgrade --from-downloaded") != 1 {
		t.Fatalf("reboots %d calls %v", e.reboots.Load(), e.fb.CallList())
	}
}

func TestAutoModes(t *testing.T) {
	ctx := context.Background()

	off := newEnv(t, ref)
	off.settings(settings.Patch{Auto: ptr(settings.AutoOff)})
	off.fb.Publish(ref, v2)
	off.clk.Set(day(3, 30))
	off.a.Tick(ctx, off.clk.Now())
	if !off.called("upgrade --check") || off.called("upgrade --download-only") || off.reboots.Load() != 0 {
		t.Fatalf("auto=off: calls %v", off.fb.CallList())
	}
	if got := off.state(); got != PhaseAvailable {
		t.Fatalf("auto=off state %s", got)
	}

	dl := newEnv(t, ref)
	dl.settings(settings.Patch{Auto: ptr(settings.AutoDownload)})
	dl.fb.Publish(ref, v2)
	dl.clk.Set(day(3, 30))
	dl.a.Tick(ctx, dl.clk.Now())
	if !dl.called("upgrade --download-only") || dl.reboots.Load() != 0 {
		t.Fatalf("auto=download: calls %v reboots %d", dl.fb.CallList(), dl.reboots.Load())
	}
	if st := dl.a.Status(ctx); st.NextWindow != nil {
		t.Fatalf("auto=download shows a window: %v", st.NextWindow)
	}

	// "Apply in the maintenance window" works with auto=download too.
	if op, err := dl.a.Apply("maintenance"); op != nil || err != nil {
		t.Fatalf("maintenance: %v %v", op, err)
	}
	dl.restart() // the request survives a restart
	dl.clk.Set(day(12, 0))
	dl.a.Tick(ctx, dl.clk.Now())
	if dl.reboots.Load() != 0 {
		t.Fatal("pending apply rebooted outside the window")
	}
	dl.clk.Set(day(4, 0).AddDate(0, 0, 1))
	dl.a.Tick(ctx, dl.clk.Now())
	if dl.reboots.Load() != 1 {
		t.Fatal("pending apply did not run in the window")
	}
	dl.restart()
	if st := dl.a.Status(ctx); st.Update.PendingApply {
		t.Fatal("pending flag left after apply")
	}
}

func TestCancelPending(t *testing.T) {
	e := newEnv(t, ref)
	e.settings(settings.Patch{Auto: ptr(settings.AutoDownload)})
	e.fb.Publish(ref, v2)
	e.a.Apply("maintenance")
	e.a.CancelPending()
	e.clk.Set(day(3, 30))
	e.a.Tick(context.Background(), e.clk.Now())
	if e.reboots.Load() != 0 {
		t.Fatal("cancelled request still applied")
	}
}

func TestSchedulerBackoffOnErrors(t *testing.T) {
	e := newEnv(t, ref)
	ctx := context.Background()
	e.fb.SetFail("upgrade --check", "error: Pulling manifest: connection refused")
	e.a.Tick(ctx, e.clk.Now())
	if st := e.a.Status(ctx); st.Update.LastError == nil || st.Update.LastError.Op != OpCheck {
		t.Fatalf("last error = %+v", st.Update.LastError)
	}
	e.clk.Set(day(14, 1))
	e.a.Tick(ctx, e.clk.Now())
	if n := e.count("upgrade --check"); n != 1 {
		t.Fatalf("retried after a minute: %d checks", n)
	}
	e.fb.SetFail("upgrade --check", "")
	e.clk.Set(day(14, 31))
	e.a.Tick(ctx, e.clk.Now())
	if n := e.count("upgrade --check"); n != 2 {
		t.Fatalf("no retry after 30 min: %d checks", n)
	}
	if st := e.a.Status(ctx); st.Update.LastError != nil {
		t.Fatalf("error not cleared: %+v", st.Update.LastError)
	}
	// Next check only after the interval (6h).
	e.clk.Set(day(20, 0))
	e.a.Tick(ctx, e.clk.Now())
	if n := e.count("upgrade --check"); n != 2 {
		t.Fatalf("checked before 6h: %d", n)
	}
	e.clk.Set(day(20, 31))
	e.a.Tick(ctx, e.clk.Now())
	if n := e.count("upgrade --check"); n != 3 {
		t.Fatalf("no check after 6h: %d", n)
	}

	// A failing download is retried only after RetryAfter.
	e.fb.Publish(ref, v2)
	e.fb.SetFail("upgrade --download-only", "error: No space left on device")
	e.clk.Set(day(23, 0))
	e.a.Check() // the periodic check is not due yet; check by hand
	e.a.Wait()
	e.a.Tick(ctx, e.clk.Now())
	e.a.Check()
	e.a.Wait()
	e.a.Tick(ctx, e.clk.Now())
	first := e.count("upgrade --download-only")
	e.clk.Set(day(23, 10))
	e.a.Tick(ctx, e.clk.Now())
	if got := e.count("upgrade --download-only"); got != first {
		t.Fatalf("download retried too early: %d -> %d", first, got)
	}
}

func TestStateSurvivesCorruptFiles(t *testing.T) {
	e := newEnv(t, ref)
	os.WriteFile(filepath.Join(e.dir, "state.json"), []byte("{"), 0o600)
	os.WriteFile(filepath.Join(e.dir, "settings.json"), []byte("nope"), 0o600)
	e.restart()
	if got := e.a.Settings(); got != settings.Default() {
		t.Fatalf("settings = %+v", got)
	}
	if got := e.state(); got != PhaseUpToDate {
		t.Fatalf("state = %s", got)
	}
}
