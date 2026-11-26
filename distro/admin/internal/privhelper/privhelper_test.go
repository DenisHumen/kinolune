// SPDX-License-Identifier: MPL-2.0

package privhelper

import (
	"context"
	"errors"
	"net"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/samba"
)

var ctx = context.Background()

// fakeAuth knows a few passwords.
type fakeAuth map[string]authhelper.Identity

const goodPW = "Right-pass-1"

func (f fakeAuth) Authenticate(user, pw string) (authhelper.Identity, error) {
	id, ok := f[user]
	if !ok || pw != goodPW {
		return authhelper.Identity{}, authhelper.ErrDenied
	}
	return id, nil
}

// call is one command the helper ran.
type call struct {
	stdin string
	argv  string
}

type fakeRunner struct {
	mu    sync.Mutex
	calls []call
	out   map[string]string // argv prefix → output
	fail  map[string]bool   // argv prefix → error
}

func (f *fakeRunner) Run(_ context.Context, stdin, name string, args ...string) (string, error) {
	argv := strings.Join(append([]string{name}, args...), " ")
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls = append(f.calls, call{stdin, argv})
	for p := range f.fail {
		if strings.HasPrefix(argv, p) {
			return "", errors.New(argv + ": exit status 1")
		}
	}
	for p, o := range f.out {
		if strings.HasPrefix(argv, p) {
			return o, nil
		}
	}
	return "", nil
}

func (f *fakeRunner) argvs() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	var out []string
	for _, c := range f.calls {
		out = append(out, c.argv)
	}
	return out
}

func (f *fakeRunner) reset() {
	f.mu.Lock()
	f.calls = nil
	f.mu.Unlock()
}

const passwd = `root:x:0:0:Super User:/root:/bin/bash
alpha-admin:x:990:990::/var/lib/alpha/admin:/usr/sbin/nologin
anna:x:1000:1000:Anna K:/home/anna:/bin/bash
boris:x:1001:1001::/home/boris:/bin/bash
carla:x:1002:1002:Carla,,,:/home/carla:/bin/bash
nobody:x:65534:65534:Kernel Overflow User:/:/sbin/nologin
`

const group = `root:x:0:
wheel:x:10:anna
alpha-admin:x:990:
alpha-users:x:991:boris
anna:x:1000:
boris:x:1001:
carla:x:1002:
media:x:1003:
`

func newHelper(t *testing.T) (*Helper, *fakeRunner, string) {
	t.Helper()
	dir := t.TempDir()
	for name, data := range map[string]string{"passwd": passwd, "group": group} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(data), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	r := &fakeRunner{
		out: map[string]string{
			"pdbedit -L":                        "anna:1000:Anna K\nboris:1001:\n",
			"systemctl is-enabled sshd.service": "disabled\n",
			"systemctl is-active sshd.service":  "inactive\n",
		},
		fail: map[string]bool{},
	}
	h := &Helper{
		Auth: fakeAuth{
			"anna":  {User: "anna", UID: 1000, Groups: []string{"anna", "wheel"}},
			"boris": {User: "boris", UID: 1001, Groups: []string{"boris", "alpha-users"}},
			"carla": {User: "carla", UID: 1002, Groups: []string{"carla"}},
		},
		Run: r,
		Paths: Paths{
			Passwd: filepath.Join(dir, "passwd"), Group: filepath.Join(dir, "group"),
			SambaConf: filepath.Join(dir, "smb.conf.d", "kinolune.conf"), Settings: filepath.Join(dir, "samba", "settings.json"),
			Import: filepath.Join(dir, "samba", "import.smbpasswd"),
			Media:  filepath.Join(dir, "srv", "media"), Users: filepath.Join(dir, "srv", "users"),
			TimeMachine: filepath.Join(dir, "srv", "timemachine"),
			RootKeys:    filepath.Join(dir, "root_keys"), HomeGlobKeys: filepath.Join(dir, "home", "*", ".ssh", "keys"),
			SSHUnit: "sshd.service",
			Lock:    filepath.Join(dir, "run", "users.lock"), UIDHigh: filepath.Join(dir, "lib", "uid-high"),
			Proc: filepath.Join(dir, "proc"),
		},
		RootUID:     os.Getuid(),
		AdminGroups: []string{"wheel", "alpha-admin"},
		MinUID:      1000,
		Now:         func() time.Time { return time.Date(2026, 11, 26, 12, 0, 0, 0, time.UTC) },
	}
	return h, r, dir
}

func bptr(b bool) *bool { return &b }

func code(err error) string {
	var oe *opError
	if errors.As(err, &oe) {
		return oe.code
	}
	if err != nil {
		return "error"
	}
	return ""
}

func TestUsers(t *testing.T) {
	h, _, _ := newHelper(t)
	resp, err := h.Handle(ctx, Request{Op: OpUsers})
	if err != nil {
		t.Fatal(err)
	}
	want := []User{
		{Name: "anna", FullName: "Anna K", UID: 1000, Admin: true, NAS: true, Samba: true},
		{Name: "boris", UID: 1001, NAS: true, Samba: true},
		{Name: "carla", FullName: "Carla", UID: 1002},
	}
	if !slices.Equal(resp.Users, want) {
		t.Fatalf("%+v", resp.Users)
	}
}

func TestCreateUser(t *testing.T) {
	h, r, dir := newHelper(t)
	req := Request{Op: OpUserCreate, Actor: "anna", ActorPassword: goodPW, User: "dina", FullName: "Dina", Password: "New-pass-12", Admin: bptr(false)}
	if _, err := h.Handle(ctx, req); err != nil {
		t.Fatal(err)
	}
	want := []string{
		"useradd -m -U -u 1003 -s /bin/bash -G alpha-users -c Dina -- dina",
		"chpasswd",
		"smbpasswd -a -s -- dina",
		"install -d -m 0700 -o dina -g dina -- " + filepath.Join(dir, "srv", "users", "dina"),
		"install -d -m 0700 -o dina -g dina -- " + filepath.Join(dir, "srv", "timemachine", "dina"),
	}
	if got := r.argvs(); !slices.Equal(got, want) {
		t.Fatalf("got\n%s", strings.Join(got, "\n"))
	}
	// Passwords go only over stdin.
	if r.calls[1].stdin != "dina:New-pass-12\n" || r.calls[2].stdin != "New-pass-12\nNew-pass-12\n" {
		t.Fatalf("stdin %q %q", r.calls[1].stdin, r.calls[2].stdin)
	}
	for _, c := range r.calls {
		if strings.Contains(c.argv, "New-pass") {
			t.Fatalf("password on a command line: %s", c.argv)
		}
	}
	r.reset()
	req.User, req.Admin = "erik", bptr(true)
	if _, err := h.Handle(ctx, req); err != nil {
		t.Fatal(err)
	}
	// The fake runner does not write passwd: the recorded UID moves on.
	if got := r.argvs()[0]; got != "useradd -m -U -u 1004 -s /bin/bash -G alpha-users,wheel -c Dina -- erik" {
		t.Fatal(got)
	}
}

func TestCreateUserRefusals(t *testing.T) {
	h, r, _ := newHelper(t)
	base := Request{Op: OpUserCreate, Actor: "anna", ActorPassword: goodPW, User: "dina", Password: "New-pass-12"}
	cases := []struct {
		name string
		mod  func(*Request)
		code string
	}{
		{"wrong admin password", func(q *Request) { q.ActorPassword = "nope" }, CodeDenied},
		{"no admin password", func(q *Request) { q.ActorPassword = "" }, CodeDenied},
		{"actor not admin", func(q *Request) { q.Actor = "boris" }, CodeDenied},
		{"bad name", func(q *Request) { q.User = "Dina;rm" }, CodeBadRequest},
		{"option-like name", func(q *Request) { q.User = "-rf" }, CodeBadRequest},
		{"reserved", func(q *Request) { q.User = "root" }, CodeBadRequest},
		{"alpha- prefix", func(q *Request) { q.User = "alpha-x" }, CodeBadRequest},
		{"taken user", func(q *Request) { q.User = "boris" }, CodeConflict},
		{"taken group", func(q *Request) { q.User = "media" }, CodeConflict},
		{"short password", func(q *Request) { q.Password = "short" }, CodeBadRequest},
		{"newline", func(q *Request) { q.Password = "Long-pass\nroot:x" }, CodeBadRequest},
		{"colon in full name", func(q *Request) { q.FullName = "a:b" }, CodeBadRequest},
	}
	for _, c := range cases {
		q := base
		c.mod(&q)
		if _, err := h.Handle(ctx, q); code(err) != c.code {
			t.Errorf("%s: %v", c.name, err)
		}
	}
	if len(r.calls) != 0 {
		t.Fatalf("ran commands: %v", r.argvs())
	}
}

func TestSetPassword(t *testing.T) {
	h, r, _ := newHelper(t)
	if _, err := h.Handle(ctx, Request{Op: OpUserPassword, Actor: "anna", ActorPassword: goodPW, User: "nosuch", Password: "Another-1"}); code(err) != CodeNotFound {
		t.Fatal(err)
	}
	if _, err := h.Handle(ctx, Request{Op: OpUserPassword, Actor: "anna", ActorPassword: goodPW, User: "boris", Password: "Another-1"}); err != nil {
		t.Fatal(err)
	}
	if got := r.argvs()[:2]; !slices.Equal(got, []string{"chpasswd", "smbpasswd -a -s -- boris"}) {
		t.Fatal(got)
	}
}

func TestSetAdminAndDelete(t *testing.T) {
	h, r, dir := newHelper(t)
	// anna is the only administrator: she cannot demote or delete herself.
	if _, err := h.Handle(ctx, Request{Op: OpUserAdmin, Actor: "anna", ActorPassword: goodPW, User: "anna", Admin: bptr(false)}); code(err) != CodeConflict {
		t.Fatal(err)
	}
	if _, err := h.Handle(ctx, Request{Op: OpUserDelete, Actor: "anna", ActorPassword: goodPW, User: "anna"}); code(err) != CodeConflict {
		t.Fatal(err)
	}
	if _, err := h.Handle(ctx, Request{Op: OpUserAdmin, Actor: "anna", ActorPassword: goodPW, User: "boris", Admin: bptr(true)}); err != nil {
		t.Fatal(err)
	}
	if got := r.argvs(); !slices.Equal(got, []string{"usermod -a -G wheel,alpha-users -- boris"}) {
		t.Fatal(got)
	}
	// Already admin: nothing to do.
	r.reset()
	if _, err := h.Handle(ctx, Request{Op: OpUserAdmin, Actor: "anna", ActorPassword: goodPW, User: "anna", Admin: bptr(true)}); err != nil || len(r.calls) != 0 {
		t.Fatal(err, r.argvs())
	}

	// Two administrators in the group file now: demoting the other works.
	g := strings.Replace(group, "wheel:x:10:anna", "wheel:x:10:anna,carla", 1)
	if err := os.WriteFile(h.Paths.Group, []byte(g), 0o644); err != nil {
		t.Fatal(err)
	}
	r.reset()
	if _, err := h.Handle(ctx, Request{Op: OpUserAdmin, Actor: "anna", ActorPassword: goodPW, User: "carla", Admin: bptr(false)}); err != nil {
		t.Fatal(err)
	}
	if got := r.argvs(); !slices.Equal(got, []string{"usermod -a -G alpha-users -- carla", "gpasswd -d carla wheel"}) {
		t.Fatal(got)
	}

	// Delete boris: Samba account, folders kept for root:wheel, then userdel.
	users := filepath.Join(dir, "srv", "users")
	if err := os.MkdirAll(filepath.Join(users, "boris"), 0o700); err != nil {
		t.Fatal(err)
	}
	r.reset()
	if _, err := h.Handle(ctx, Request{Op: OpUserDelete, Actor: "anna", ActorPassword: goodPW, User: "boris"}); err != nil {
		t.Fatal(err)
	}
	kept := filepath.Join(users, "deleted-boris-20261126-120000")
	want := []string{
		"pdbedit -x -u boris",
		"smbstatus --brief --json",
		"mv -T -- " + filepath.Join(users, "boris") + " " + kept,
		"chown -R --no-dereference root:wheel -- " + kept,
		"chmod -R u+rwX,g+rwX,o-rwx -- " + kept,
		"userdel -r -- boris",
	}
	if got := r.argvs(); !slices.Equal(got, want) {
		t.Fatalf("got\n%s", strings.Join(got, "\n"))
	}
	// A wrong administrator password changes nothing.
	r.reset()
	if _, err := h.Handle(ctx, Request{Op: OpUserDelete, Actor: "anna", ActorPassword: "x", User: "carla"}); code(err) != CodeDenied || len(r.calls) != 0 {
		t.Fatal(err, r.argvs())
	}
}

func TestSambaSync(t *testing.T) {
	h, r, _ := newHelper(t)
	if _, err := h.Handle(ctx, Request{Op: OpSambaSync, User: "boris", Password: "wrong"}); code(err) != CodeDenied {
		t.Fatal(err)
	}
	// carla is no NAS user: nothing to sync.
	if _, err := h.Handle(ctx, Request{Op: OpSambaSync, User: "carla", Password: goodPW}); err != nil || len(r.calls) != 0 {
		t.Fatal(err, r.argvs())
	}
	if _, err := h.Handle(ctx, Request{Op: OpSambaSync, User: "boris", Password: goodPW}); err != nil {
		t.Fatal(err)
	}
	if got := r.argvs(); len(got) != 3 || got[0] != "smbpasswd -a -s -- boris" || r.calls[0].stdin != goodPW+"\n"+goodPW+"\n" {
		t.Fatal(got)
	}
}

func TestSharing(t *testing.T) {
	h, r, _ := newHelper(t)
	resp, err := h.Handle(ctx, Request{Op: OpSharingGet})
	if err != nil || *resp.Settings != (samba.Settings{}) {
		t.Fatal(resp, err)
	}
	on := samba.Settings{MediaGuestRead: true}
	if _, err := h.Handle(ctx, Request{Op: OpSharingSet, Settings: &on}); err != nil {
		t.Fatal(err)
	}
	conf, _ := os.ReadFile(h.Paths.SambaConf)
	if string(conf) != samba.Render(on) {
		t.Fatal("conf not rendered")
	}
	if got := r.argvs(); !slices.Equal(got, []string{"testparm -s --suppress-prompt", "smbcontrol smbd reload-config"}) {
		t.Fatal(got)
	}
	// testparm refuses: settings and file roll back.
	r.fail["testparm"] = true
	off := samba.Settings{TimeMachineOff: true}
	if _, err := h.Handle(ctx, Request{Op: OpSharingSet, Settings: &off}); err == nil {
		t.Fatal("accepted")
	}
	if s, _ := samba.LoadSettings(h.Paths.Settings); s != on {
		t.Fatalf("settings not rolled back: %+v", s)
	}
	if conf2, _ := os.ReadFile(h.Paths.SambaConf); string(conf2) != string(conf) {
		t.Fatal("conf not rolled back")
	}
}

func TestRenderBoot(t *testing.T) {
	h, r, dir := newHelper(t)
	if err := os.MkdirAll(filepath.Dir(h.Paths.Import), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(h.Paths.Import, []byte("anna:1000:X:Y:[U          ]:LCT-00000000:\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := h.RenderBoot(ctx); err != nil {
		t.Fatal(err)
	}
	if conf, _ := os.ReadFile(h.Paths.SambaConf); string(conf) != samba.Render(samba.Settings{}) {
		t.Fatal("conf")
	}
	if _, err := os.Stat(h.Paths.Import); !os.IsNotExist(err) {
		t.Fatal("import file left behind")
	}
	got := r.argvs()
	if got[0] != "pdbedit -i smbpasswd:"+h.Paths.Import+" -e tdbsam" {
		t.Fatal(got)
	}
	// Folders for anna (wheel) and boris (alpha-users), not carla.
	joined := strings.Join(got, "\n")
	for _, u := range []string{"anna", "boris"} {
		if !strings.Contains(joined, filepath.Join(dir, "srv", "users", u)) {
			t.Errorf("no folder for %s", u)
		}
	}
	if strings.Contains(joined, "carla") {
		t.Error("folder for carla")
	}
}

func TestSSH(t *testing.T) {
	h, r, dir := newHelper(t)
	if err := os.WriteFile(h.Paths.RootKeys, []byte("ssh-ed25519 AAAA a\n\n# c\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(dir, "home", "anna", ".ssh"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "home", "anna", ".ssh", "keys"), []byte("ssh-ed25519 BBBB b\nssh-rsa CCCC c\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	resp, err := h.Handle(ctx, Request{Op: OpSSHGet})
	if err != nil || *resp.SSH != (SSHState{AuthorizedKeys: 3}) {
		t.Fatal(resp.SSH, err)
	}
	if _, err := h.Handle(ctx, Request{Op: OpSSHSet, Enabled: bptr(true)}); err != nil {
		t.Fatal(err)
	}
	if !slices.Contains(r.argvs(), "systemctl enable --now sshd.service") {
		t.Fatal(r.argvs())
	}
	if _, err := h.Handle(ctx, Request{Op: OpSSHSet}); code(err) != CodeBadRequest {
		t.Fatal(err)
	}
}

// TestProtocol runs Serve and Client over a real unix socket.
func TestProtocol(t *testing.T) {
	h, _, dir := newHelper(t)
	sock := filepath.Join(dir, "priv.sock")
	ln, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			_ = h.Serve(ctx, c)
			c.Close()
		}
	}()
	c := &Client{Socket: sock, Timeout: 5 * time.Second}
	resp, err := c.Do(ctx, Request{Op: OpUsers})
	if err != nil || len(resp.Users) != 3 {
		t.Fatal(resp, err)
	}
	_, err = c.Do(ctx, Request{Op: OpUserDelete, Actor: "anna", ActorPassword: "bad", User: "boris"})
	if !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
	_, err = c.Do(ctx, Request{Op: OpUserPassword, Actor: "anna", ActorPassword: goodPW, User: "zed", Password: "Long-enough-1"})
	if !errors.Is(err, ErrNotFound) {
		t.Fatal(err)
	}
	_, err = c.Do(ctx, Request{Op: "rm -rf"})
	if !errors.Is(err, ErrBadRequest) {
		t.Fatal(err)
	}
	if _, err := (&Client{Socket: filepath.Join(dir, "none.sock")}).Do(ctx, Request{Op: OpUsers}); !errors.Is(err, ErrUnavailable) {
		t.Fatal(err)
	}
}
