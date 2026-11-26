// SPDX-License-Identifier: MPL-2.0

package privhelper

import (
	"context"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
)

// groupRunner applies "gpasswd -d USER GROUP" to the group file, slowly, so
// two helpers that do not serialise would both pass the last-admin check.
type groupRunner struct {
	*fakeRunner
	group string
	mu    sync.Mutex
}

func (g *groupRunner) Run(ctx context.Context, stdin, name string, args ...string) (string, error) {
	if name == "gpasswd" && len(args) == 3 && args[0] == "-d" {
		time.Sleep(50 * time.Millisecond)
		g.mu.Lock()
		b, _ := os.ReadFile(g.group)
		var out []string
		for _, l := range strings.Split(strings.TrimRight(string(b), "\n"), "\n") {
			f := strings.Split(l, ":")
			if len(f) == 4 && f[0] == args[2] {
				f[3] = strings.Join(slices.DeleteFunc(strings.Split(f[3], ","), func(m string) bool { return m == args[1] }), ",")
			}
			out = append(out, strings.Join(f, ":"))
		}
		_ = os.WriteFile(g.group, []byte(strings.Join(out, "\n")+"\n"), 0o644)
		g.mu.Unlock()
	}
	return g.fakeRunner.Run(ctx, stdin, name, args...)
}

// TestLastAdminRace: anna and carla demote each other at the same moment in
// two helper "processes"; the users lock lets exactly one win.
func TestLastAdminRace(t *testing.T) {
	h1, r, _ := newHelper(t)
	g := strings.Replace(group, "wheel:x:10:anna", "wheel:x:10:anna,carla", 1)
	if err := os.WriteFile(h1.Paths.Group, []byte(g), 0o644); err != nil {
		t.Fatal(err)
	}
	auth := h1.Auth.(fakeAuth)
	auth["carla"] = authhelper.Identity{User: "carla", UID: 1002, Groups: []string{"carla", "wheel"}}
	gr := &groupRunner{fakeRunner: r, group: h1.Paths.Group}
	h1.Run = gr
	h2 := *h1 // a second helper process: same files, same lock path
	var wg sync.WaitGroup
	errs := make([]error, 2)
	for i, pair := range [][2]string{{"anna", "carla"}, {"carla", "anna"}} {
		wg.Add(1)
		h := h1
		if i == 1 {
			h = &h2
		}
		go func() {
			defer wg.Done()
			_, errs[i] = h.Handle(ctx, Request{Op: OpUserAdmin, Actor: pair[0], ActorPassword: goodPW, User: pair[1], Admin: bptr(false)})
		}()
	}
	wg.Wait()
	codes := []string{code(errs[0]), code(errs[1])}
	slices.Sort(codes)
	if !slices.Equal(codes, []string{"", CodeConflict}) {
		t.Fatalf("results %v (%v)", codes, errs)
	}
	b, _ := os.ReadFile(h1.Paths.Group)
	if !strings.Contains(string(b), "wheel:x:10:anna\n") && !strings.Contains(string(b), "wheel:x:10:carla\n") {
		t.Fatalf("not exactly one administrator left:\n%s", b)
	}
}

// TestUIDNotReused: a deleted user's UID is never handed out again.
func TestUIDNotReused(t *testing.T) {
	h, r, _ := newHelper(t)
	// carla (1002) has the highest UID; delete her, then create dina.
	if _, err := h.Handle(ctx, Request{Op: OpUserDelete, Actor: "anna", ActorPassword: goodPW, User: "carla"}); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(h.Paths.Passwd, []byte(strings.Replace(passwd, "carla:x:1002:1002:Carla,,,:/home/carla:/bin/bash\n", "", 1)), 0o644); err != nil {
		t.Fatal(err)
	}
	r.reset()
	if _, err := h.Handle(ctx, Request{Op: OpUserCreate, Actor: "anna", ActorPassword: goodPW, User: "dina", Password: "Dina-pass-12"}); err != nil {
		t.Fatal(err)
	}
	if got := r.argvs()[0]; !strings.Contains(got, " -u 1003 ") {
		t.Fatalf("UID reused: %s", got)
	}
	if b, _ := os.ReadFile(h.Paths.UIDHigh); strings.TrimSpace(string(b)) != "1003" {
		t.Fatalf("uid-high %q", b)
	}
}

// TestKillSessions: the smbd processes serving the user end on delete and
// on a password change; other users' and non-smbd pids are left alone.
func TestKillSessions(t *testing.T) {
	h, r, dir := newHelper(t)
	for pid, comm := range map[string]string{"4101": "smbd[10.0.0.5]", "4102": "smbd", "4103": "bash", "4104": "smbd"} {
		p := filepath.Join(dir, "proc", pid)
		if err := os.MkdirAll(p, 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(p, "comm"), []byte(comm+"\n"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	r.out["smbstatus --brief --json"] = `{"timestamp":"x","version":"4.25.1","sessions":{
	 "1":{"server_id":{"pid":"4101","task_id":"0"},"uid":1001,"username":"boris"},
	 "2":{"server_id":{"pid":"4102","task_id":"0"},"uid":1001,"username":"WORKGROUP\\boris"},
	 "3":{"server_id":{"pid":"4103","task_id":"0"},"uid":1001,"username":"boris"},
	 "4":{"server_id":{"pid":"4104","task_id":"0"},"uid":1000,"username":"anna"}}}`
	if _, err := h.Handle(ctx, Request{Op: OpUserPassword, Actor: "anna", ActorPassword: goodPW, User: "boris", Password: "Boris-new-12"}); err != nil {
		t.Fatal(err)
	}
	var kills []string
	for _, a := range r.argvs() {
		if strings.HasPrefix(a, "kill ") {
			kills = append(kills, a)
		}
	}
	if !slices.Equal(kills, []string{"kill -s TERM -- 4101", "kill -s TERM -- 4102"}) {
		t.Fatalf("kills %v", kills)
	}
	if got := sessionPIDs("not json", "boris"); got != nil {
		t.Fatal(got)
	}
}

// TestKeysSafe: symlinks, FIFOs, files of another owner are not read; big
// files are read only up to the cap.
func TestKeysSafe(t *testing.T) {
	h, _, dir := newHelper(t)
	secret := filepath.Join(dir, "shadow")
	if err := os.WriteFile(secret, []byte("a\nb\nc\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(secret, h.Paths.RootKeys); err != nil {
		t.Fatal(err)
	}
	if n := h.countKeys(); n != 0 {
		t.Fatalf("followed a symlink: %d", n)
	}
	os.Remove(h.Paths.RootKeys)
	if err := syscall.Mkfifo(h.Paths.RootKeys, 0o600); err != nil {
		t.Fatal(err)
	}
	done := make(chan int)
	go func() { done <- h.countKeys() }()
	select {
	case n := <-done:
		if n != 0 {
			t.Fatalf("fifo: %d", n)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("hangs on a FIFO")
	}
	os.Remove(h.Paths.RootKeys)
	big := strings.Repeat("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx k\n", 40000) // ~3.5 MiB
	if err := os.WriteFile(h.Paths.RootKeys, []byte(big), 0o600); err != nil {
		t.Fatal(err)
	}
	perLine := len(big) / 40000
	if n := h.countKeys(); n > maxKeysFile/perLine+1 || n < maxKeysFile/perLine-1 {
		t.Fatalf("read past the cap: %d keys", n)
	}
	// Owned by someone else than root (here: the test's own UID is "root").
	h.RootUID = os.Getuid() + 1
	if n := h.countKeys(); n != 0 {
		t.Fatalf("file of another owner read: %d", n)
	}
}
