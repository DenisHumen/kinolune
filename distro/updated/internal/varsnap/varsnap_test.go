// SPDX-License-Identifier: MPL-2.0

package varsnap

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// fakeFS emulates btrfs with plain directories: the "top level" is already
// there at MountDir, a snapshot is a recursive copy.
type fakeFS struct {
	ro       map[string]bool
	mounted  int
	failSnap bool
}

func (f *fakeFS) MountTop(_ context.Context, dev, dir string) error {
	if dev != "/dev/vda3" {
		return errors.New("wrong device " + dev)
	}
	f.mounted++
	return nil
}
func (f *fakeFS) Unmount(context.Context, string) error { f.mounted--; return nil }
func (f *fakeFS) Snapshot(_ context.Context, src, dst string, readonly bool) error {
	if f.failSnap {
		return errors.New("no space left on device")
	}
	if err := os.CopyFS(dst, os.DirFS(src)); err != nil {
		return err
	}
	f.ro[dst] = readonly
	return nil
}
func (f *fakeFS) SetReadOnly(_ context.Context, p string) error { f.ro[p] = true; return nil }
func (f *fakeFS) Delete(_ context.Context, p string) error      { delete(f.ro, p); return os.RemoveAll(p) }

type env struct {
	m   *Manager
	fs  *fakeFS
	top string
	now time.Time
}

func newEnv(t *testing.T) *env {
	t.Helper()
	dir := t.TempDir()
	top := filepath.Join(dir, "top")
	must(t, os.MkdirAll(filepath.Join(top, "@var", "lib", "alpha-media"), 0o755))
	writeDB(t, top, "schema=1")
	fstab := filepath.Join(dir, "fstab")
	must(t, os.WriteFile(fstab, []byte("# test\nUUID=abc / btrfs defaults 0 0\nUUID=abc /var btrfs subvol=/@var,compress=zstd:1 0 0\n"), 0o644))
	mi := filepath.Join(dir, "mountinfo")
	must(t, os.WriteFile(mi, []byte(
		"22 1 0:20 / / rw - overlay overlay rw\n"+
			"30 22 0:31 / /sysroot ro,relatime shared:4 - btrfs /dev/vda3 ro,compress=zstd:1\n"), 0o644))
	e := &env{fs: &fakeFS{ro: map[string]bool{}}, top: top, now: time.Date(2026, 11, 26, 3, 0, 0, 0, time.UTC)}
	e.m = &Manager{Fstab: fstab, Mountinfo: mi, MountDir: top, FS: e.fs, Now: func() time.Time { return e.now }}
	return e
}

func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func writeDB(t *testing.T, top, v string) {
	t.Helper()
	must(t, os.WriteFile(filepath.Join(top, "@var", "lib", "alpha-media", "library.db"), []byte(v), 0o600))
}

func readDB(t *testing.T, top string) string {
	t.Helper()
	b, err := os.ReadFile(filepath.Join(top, "@var", "lib", "alpha-media", "library.db"))
	must(t, err)
	return string(b)
}

func (e *env) prepare(t *testing.T) Pending {
	t.Helper()
	p, err := e.m.Prepare(context.Background(), Pending{FromChecksum: "old", FromVersion: "1", ToChecksum: "new", ToVersion: "2", ToDigest: "sha256:2"})
	must(t, err)
	e.now = e.now.Add(time.Hour)
	return p
}

func (e *env) boot(t *testing.T, booted string) BootAction {
	t.Helper()
	act, err := e.m.Boot(context.Background(), booted)
	must(t, err)
	return act
}

func TestFailedUpdateRestoresVar(t *testing.T) {
	e := newEnv(t)
	p := e.prepare(t)
	if p.Snapshot != "20261126T030000Z" || !e.fs.ro[filepath.Join(e.top, "@var-snapshots", p.Snapshot)] {
		t.Fatalf("snapshot %q ro=%v", p.Snapshot, e.fs.ro)
	}
	// Reboot into the update; its media server migrates the database.
	if act := e.boot(t, "new"); act != BootToBooted {
		t.Fatalf("first boot of the update: %s", act)
	}
	writeDB(t, e.top, "schema=2")
	if act := e.boot(t, "new"); act != BootUnchanged {
		t.Fatalf("second boot of the update: %s", act)
	}
	// greenboot rolls the OS back: /var comes back before it is mounted.
	if act := e.boot(t, "old"); act != BootRestored {
		t.Fatalf("rollback boot: %s", act)
	}
	if got := readDB(t, e.top); got != "schema=1" {
		t.Fatalf("database after restore: %q", got)
	}
	st, err := e.m.State(context.Background())
	must(t, err)
	if st.Pending != nil || st.LastRestore == nil || st.LastRestore.Snapshot != p.Snapshot || st.LastRestore.Version != "2" {
		t.Fatalf("state after restore: %+v", st)
	}
	// The replaced /var is kept read-only: nothing is lost.
	repl := filepath.Join(e.top, "@var-snapshots", st.LastRestore.Replaced)
	b, err := os.ReadFile(filepath.Join(repl, "lib", "alpha-media", "library.db"))
	must(t, err)
	if string(b) != "schema=2" || !e.fs.ro[repl] {
		t.Fatalf("replaced /var: %q ro=%v", b, e.fs.ro[repl])
	}
	if len(st.Snapshots) != 2 || st.Snapshots[1].Kind != KindReplaced {
		t.Fatalf("snapshots: %+v", st.Snapshots)
	}
	// The restored @var is writable; the next boot does nothing.
	if e.fs.ro[filepath.Join(e.top, "@var")] {
		t.Fatal("restored @var is read-only")
	}
	if act := e.boot(t, "old"); act != BootNothing {
		t.Fatalf("boot after restore: %s", act)
	}
	if e.fs.mounted != 0 {
		t.Fatalf("top level left mounted %d times", e.fs.mounted)
	}
}

func TestConfirmedUpdateKeepsVar(t *testing.T) {
	e := newEnv(t)
	e.prepare(t)
	e.boot(t, "new")
	writeDB(t, e.top, "schema=2")
	must(t, e.m.Confirm(context.Background()))
	// A manual rollback later keeps the state (expand→contract, LIB-002).
	if act := e.boot(t, "old"); act != BootNothing {
		t.Fatalf("rollback after confirm: %s", act)
	}
	if got := readDB(t, e.top); got != "schema=2" {
		t.Fatalf("database: %q", got)
	}
}

func TestUpdateThatNeverBootedDropsMarker(t *testing.T) {
	e := newEnv(t)
	e.prepare(t)
	writeDB(t, e.top, "schema=1, new rows") // the old version kept working
	if act := e.boot(t, "old"); act != BootDropped {
		t.Fatalf("boot: %s", act)
	}
	if got := readDB(t, e.top); got != "schema=1, new rows" {
		t.Fatalf("database: %q", got)
	}
	st, err := e.m.State(context.Background())
	must(t, err)
	if st.Pending != nil || st.LastRestore != nil {
		t.Fatalf("state: %+v", st)
	}
}

func TestUnknownBootedKeepsMarker(t *testing.T) {
	e := newEnv(t)
	e.prepare(t)
	if act := e.boot(t, ""); act != BootUnchanged {
		t.Fatalf("boot: %s", act)
	}
}

func TestKeepsThreeSnapshots(t *testing.T) {
	e := newEnv(t)
	var last Pending
	for i := 0; i < 5; i++ {
		last = e.prepare(t)
	}
	st, err := e.m.State(context.Background())
	must(t, err)
	if len(st.Snapshots) != 3 || st.Snapshots[2].Name != last.Snapshot || st.Snapshots[0].Name != "20261126T050000Z" {
		t.Fatalf("snapshots: %+v", st.Snapshots)
	}
	if _, err := os.Stat(filepath.Join(e.top, "@var-snapshots", "20261126T030000Z.json")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("metadata of a deleted snapshot stays: %v", err)
	}
}

func TestSnapshotFailure(t *testing.T) {
	e := newEnv(t)
	e.fs.failSnap = true
	if _, err := e.m.Prepare(context.Background(), Pending{FromChecksum: "old", ToChecksum: "new"}); err == nil {
		t.Fatal("no error")
	}
	if e.fs.mounted != 0 {
		t.Fatal("top level left mounted")
	}
	e.fs.failSnap = false
	st, err := e.m.State(context.Background())
	must(t, err)
	if st.Pending != nil {
		t.Fatal("marker written without a snapshot")
	}
}

func TestRestoreFailurePutsVarBack(t *testing.T) {
	e := newEnv(t)
	e.prepare(t)
	e.boot(t, "new")
	writeDB(t, e.top, "schema=2")
	e.fs.failSnap = true
	if _, err := e.m.Boot(context.Background(), "old"); err == nil {
		t.Fatal("no error")
	}
	if got := readDB(t, e.top); got != "schema=2" {
		t.Fatalf("@var not put back: %q", got)
	}
}

func TestNoLayout(t *testing.T) {
	e := newEnv(t)
	must(t, os.WriteFile(e.m.Fstab, []byte("UUID=abc / xfs defaults 0 0\n"), 0o644))
	if _, err := e.m.Prepare(context.Background(), Pending{}); !errors.Is(err, ErrNoLayout) {
		t.Fatalf("err = %v", err)
	}
	must(t, os.WriteFile(e.m.Fstab, []byte("UUID=abc /var xfs defaults 0 0\n"), 0o644))
	if _, err := e.m.Boot(context.Background(), "old"); !errors.Is(err, ErrNoLayout) {
		t.Fatalf("err = %v", err)
	}
}

func TestMountSourceEscapes(t *testing.T) {
	mi := filepath.Join(t.TempDir(), "mi")
	must(t, os.WriteFile(mi, []byte("30 22 0:31 / /sys\\040root ro - btrfs /dev/disk\\040x ro\n"), 0o644))
	got, err := MountSource(mi, "/sys root", "btrfs")
	if err != nil || got != "/dev/disk x" {
		t.Fatalf("got %q, %v", got, err)
	}
}

func TestBootedChecksum(t *testing.T) {
	dir := t.TempDir()
	sys := filepath.Join(dir, "sysroot")
	dep := filepath.Join(sys, "ostree", "deploy", "default", "deploy", "a1b2c3.0")
	must(t, os.MkdirAll(dep, 0o755))
	must(t, os.MkdirAll(filepath.Join(sys, "ostree", "boot.1.1", "default", "ffee"), 0o755))
	must(t, os.Symlink("boot.1.1", filepath.Join(sys, "ostree", "boot.1")))
	must(t, os.Symlink("../../../deploy/default/deploy/a1b2c3.0", filepath.Join(sys, "ostree", "boot.1.1", "default", "ffee", "0")))
	cl := filepath.Join(dir, "cmdline")
	must(t, os.WriteFile(cl, []byte("BOOT_IMAGE=(hd0,gpt2)/boot/vmlinuz root=UUID=abc rw ostree=/ostree/boot.1/default/ffee/0\n"), 0o644))
	got, err := BootedChecksum(cl, sys)
	if err != nil || got != "a1b2c3" {
		t.Fatalf("got %q, %v", got, err)
	}
}
