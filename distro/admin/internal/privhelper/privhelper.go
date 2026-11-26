// SPDX-License-Identifier: MPL-2.0

// Package privhelper is the root helper of the admin backend for what has no
// D-Bus API with polkit: OS users of the NAS, the Samba password database
// and configuration, and SSH on/off (DIST-018, ADR-NNNN).
//
// The helper (cmd/alpha-admin-priv) is started by systemd for each
// connection on /run/alpha-admin/priv.sock (alpha-admin-priv.socket,
// Accept=yes, socket 0600 alpha-admin, SO_PEERCRED checked again), exactly
// like alpha-admin-auth. It accepts only the fixed operations below, never
// commands, paths or configuration text. Changes to users need the password
// of an administrator (the acting one), which the helper checks with PAM
// itself, so a compromised web daemon alone cannot create administrators.
//
// Protocol: one JSON line each way (Request, Response).
package privhelper

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/samba"
)

// Operations.
const (
	OpUsers        = "users"
	OpUserCreate   = "user_create"
	OpUserPassword = "user_password"
	OpUserAdmin    = "user_admin"
	OpUserDelete   = "user_delete"
	OpSambaSync    = "samba_sync"
	OpSharingGet   = "sharing_get"
	OpSharingSet   = "sharing_set"
	OpSSHGet       = "ssh_get"
	OpSSHSet       = "ssh_set"
)

// Error codes of Response.Error.
const (
	CodeDenied     = "denied"      // the administrator's password is wrong
	CodeBadRequest = "bad_request" // invalid name, password, field
	CodeNotFound   = "not_found"   // no such user
	CodeConflict   = "conflict"    // exists, last administrator, yourself
	CodeFailed     = "failed"      // a system command failed
)

// Password limits.
const (
	MinPassword = 8
	MaxPassword = authhelper.MaxPassword
	maxFullName = 64
	maxLine     = 16 << 10
)

// Request is what the daemon sends.
type Request struct {
	Op string `json:"op"`
	// Actor and ActorPassword: the administrator who asks (user changes).
	Actor         string          `json:"actor,omitempty"`
	ActorPassword string          `json:"actor_password,omitempty"`
	User          string          `json:"user,omitempty"`
	FullName      string          `json:"full_name,omitempty"`
	Password      string          `json:"password,omitempty"`
	Admin         *bool           `json:"admin,omitempty"`
	Settings      *samba.Settings `json:"settings,omitempty"`
	Enabled       *bool           `json:"enabled,omitempty"`
}

// User is a person's account on the NAS.
type User struct {
	Name     string `json:"name"`
	FullName string `json:"full_name"`
	UID      int    `json:"uid"`
	// Admin: member of an administrator group (wheel, alpha-admin).
	Admin bool `json:"admin"`
	// NAS: may use the shares (alpha-users, wheel, alpha-admin).
	NAS bool `json:"nas"`
	// Samba: has a Samba password, i.e. can log in to the shares.
	Samba bool `json:"samba"`
}

// SSHState is the SSH server's state.
type SSHState struct {
	Enabled bool `json:"enabled"`
	Active  bool `json:"active"`
	// AuthorizedKeys counts keys of root and of the users (keys only:
	// password log-in is off).
	AuthorizedKeys int `json:"authorized_keys"`
}

// Response is what the helper answers.
type Response struct {
	OK       bool            `json:"ok"`
	Error    string          `json:"error,omitempty"`
	Message  string          `json:"message,omitempty"`
	Users    []User          `json:"users,omitempty"`
	Settings *samba.Settings `json:"settings,omitempty"`
	SSH      *SSHState       `json:"ssh,omitempty"`
}

// Runner runs a system command without a shell; stdin carries secrets and
// is never logged.
type Runner interface {
	Run(ctx context.Context, stdin string, name string, args ...string) (string, error)
}

// ExecRunner runs real commands.
type ExecRunner struct{}

// Run implements Runner.
func (ExecRunner) Run(ctx context.Context, stdin, name string, args ...string) (string, error) {
	cmd := exec.CommandContext(ctx, name, args...)
	cmd.Stdin = strings.NewReader(stdin)
	cmd.Env = []string{"PATH=/usr/sbin:/usr/bin", "LANG=C.UTF-8"}
	out, err := cmd.CombinedOutput()
	if err != nil {
		msg := strings.TrimSpace(string(out))
		if len(msg) > 400 {
			msg = msg[:400]
		}
		return string(out), fmt.Errorf("%s %s: %v: %s", name, strings.Join(args, " "), err, msg)
	}
	return string(out), nil
}

// Paths of the files the helper reads and writes.
type Paths struct {
	Passwd, Group string
	SambaConf     string
	Settings      string
	Import        string
	Media, Users  string
	TimeMachine   string
	RootKeys      string // /root/.ssh/authorized_keys
	HomeGlobKeys  string // /var/home/*/.ssh/authorized_keys
	SSHUnit       string
}

// DefaultPaths are the paths in the image.
var DefaultPaths = Paths{
	Passwd:       "/etc/passwd",
	Group:        "/etc/group",
	SambaConf:    samba.ConfPath,
	Settings:     samba.SettingsPath,
	Import:       samba.ImportPath,
	Media:        samba.MediaDir,
	Users:        samba.UsersDir,
	TimeMachine:  samba.TimeMachDir,
	RootKeys:     "/root/.ssh/authorized_keys",
	HomeGlobKeys: "/var/home/*/.ssh/authorized_keys",
	SSHUnit:      "sshd.service",
}

// Helper executes requests.
type Helper struct {
	Auth        authhelper.Authenticator
	Run         Runner
	Paths       Paths
	AdminGroups []string
	MinUID      int
	Now         func() time.Time
	Log         *slog.Logger
}

// New returns a helper for the real system.
func New(a authhelper.Authenticator, log *slog.Logger) *Helper {
	return &Helper{Auth: a, Run: ExecRunner{}, Paths: DefaultPaths,
		AdminGroups: []string{"wheel", "alpha-admin"}, MinUID: 1000, Log: log}
}

func (h *Helper) now() time.Time {
	if h.Now != nil {
		return h.Now()
	}
	return time.Now()
}

func (h *Helper) log() *slog.Logger {
	if h.Log != nil {
		return h.Log
	}
	return slog.New(slog.NewTextHandler(io.Discard, nil))
}

// opError is an error with a response code.
type opError struct{ code, msg string }

func (e *opError) Error() string { return e.code + ": " + e.msg }

func fail(code, format string, args ...any) error {
	return &opError{code: code, msg: fmt.Sprintf(format, args...)}
}

// Serve answers one request on rw.
func (h *Helper) Serve(ctx context.Context, rw io.ReadWriter) error {
	resp := h.handleLine(ctx, rw)
	b, err := json.Marshal(resp)
	if err != nil {
		return err
	}
	_, err = rw.Write(append(b, '\n'))
	return err
}

func (h *Helper) handleLine(ctx context.Context, r io.Reader) Response {
	line, err := bufio.NewReaderSize(io.LimitReader(r, maxLine), maxLine).ReadBytes('\n')
	if err != nil && !(errors.Is(err, io.EOF) && len(line) > 0) {
		return Response{Error: CodeBadRequest, Message: "no request"}
	}
	var req Request
	dec := json.NewDecoder(strings.NewReader(string(line)))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&req); err != nil {
		return Response{Error: CodeBadRequest, Message: "bad request"}
	}
	resp, err := h.Handle(ctx, req)
	if err != nil {
		var oe *opError
		if errors.As(err, &oe) {
			return Response{Error: oe.code, Message: oe.msg}
		}
		h.log().Error("request failed", "op", req.Op, "err", err)
		return Response{Error: CodeFailed, Message: err.Error()}
	}
	resp.OK = true
	return resp
}

// Handle executes one request.
func (h *Helper) Handle(ctx context.Context, req Request) (Response, error) {
	switch req.Op {
	case OpUsers:
		users, err := h.users(ctx)
		return Response{Users: users}, err
	case OpUserCreate:
		return Response{}, h.createUser(ctx, req)
	case OpUserPassword:
		return Response{}, h.setPassword(ctx, req)
	case OpUserAdmin:
		return Response{}, h.setAdmin(ctx, req)
	case OpUserDelete:
		return Response{}, h.deleteUser(ctx, req)
	case OpSambaSync:
		return Response{}, h.sambaSync(ctx, req)
	case OpSharingGet:
		s, err := samba.LoadSettings(h.Paths.Settings)
		return Response{Settings: &s}, err
	case OpSharingSet:
		if req.Settings == nil {
			return Response{}, fail(CodeBadRequest, "settings missing")
		}
		return Response{Settings: req.Settings}, h.setSharing(ctx, *req.Settings)
	case OpSSHGet:
		st, err := h.ssh(ctx)
		return Response{SSH: &st}, err
	case OpSSHSet:
		if req.Enabled == nil {
			return Response{}, fail(CodeBadRequest, "enabled missing")
		}
		if err := h.setSSH(ctx, *req.Enabled); err != nil {
			return Response{}, err
		}
		st, err := h.ssh(ctx)
		return Response{SSH: &st}, err
	default:
		return Response{}, fail(CodeBadRequest, "unknown operation %q", req.Op)
	}
}

// --- accounts database -----------------------------------------------------------------

type account struct {
	name, fullName string
	uid            int
}

func readColon(path string) ([][]string, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var out [][]string
	for _, l := range strings.Split(string(b), "\n") {
		if l == "" || strings.HasPrefix(l, "#") {
			continue
		}
		out = append(out, strings.Split(l, ":"))
	}
	return out, nil
}

// humans returns accounts with MinUID <= uid < 60000, and the members of
// each group.
func (h *Helper) humans() ([]account, map[string][]string, error) {
	pw, err := readColon(h.Paths.Passwd)
	if err != nil {
		return nil, nil, err
	}
	var accts []account
	for _, r := range pw {
		if len(r) < 5 {
			continue
		}
		uid, err := strconv.Atoi(r[2])
		if err != nil || uid < h.MinUID || uid >= 60000 {
			continue
		}
		full, _, _ := strings.Cut(r[4], ",")
		accts = append(accts, account{name: r[0], fullName: full, uid: uid})
	}
	gr, err := readColon(h.Paths.Group)
	if err != nil {
		return nil, nil, err
	}
	members := map[string][]string{}
	for _, r := range gr {
		if len(r) < 4 || r[3] == "" {
			continue
		}
		members[r[0]] = strings.Split(r[3], ",")
	}
	return accts, members, nil
}

func inAny(members map[string][]string, groups []string, name string) bool {
	for _, g := range groups {
		if slices.Contains(members[g], name) {
			return true
		}
	}
	return false
}

// existsName reports whether name is taken by any user or group.
func (h *Helper) existsName(name string) (bool, error) {
	for _, f := range []string{h.Paths.Passwd, h.Paths.Group} {
		recs, err := readColon(f)
		if err != nil {
			return false, err
		}
		for _, r := range recs {
			if r[0] == name {
				return true, nil
			}
		}
	}
	return false, nil
}

func (h *Helper) sambaUsers(ctx context.Context) (map[string]bool, error) {
	out, err := h.Run.Run(ctx, "", "pdbedit", "-L")
	if err != nil {
		return nil, err
	}
	m := map[string]bool{}
	for _, l := range strings.Split(out, "\n") {
		if name, _, ok := strings.Cut(l, ":"); ok && authhelper.ValidUserName(name) {
			m[name] = true
		}
	}
	return m, nil
}

func (h *Helper) users(ctx context.Context) ([]User, error) {
	accts, members, err := h.humans()
	if err != nil {
		return nil, err
	}
	sam, err := h.sambaUsers(ctx)
	if err != nil {
		h.log().Warn("pdbedit -L", "err", err)
		sam = map[string]bool{}
	}
	users := make([]User, 0, len(accts))
	for _, a := range accts {
		users = append(users, User{
			Name: a.name, FullName: a.fullName, UID: a.uid,
			Admin: inAny(members, h.AdminGroups, a.name),
			NAS:   inAny(members, samba.NASGroups, a.name),
			Samba: sam[a.name],
		})
	}
	return users, nil
}

func (h *Helper) human(name string) (account, map[string][]string, []account, error) {
	accts, members, err := h.humans()
	if err != nil {
		return account{}, nil, nil, err
	}
	i := slices.IndexFunc(accts, func(a account) bool { return a.name == name })
	if i < 0 {
		return account{}, nil, nil, fail(CodeNotFound, "no such user %q", name)
	}
	return accts[i], members, accts, nil
}

// --- checks ----------------------------------------------------------------------------

// verifyAdmin checks the acting administrator's password with PAM.
func (h *Helper) verifyAdmin(req Request) error {
	if !authhelper.ValidUserName(req.Actor) || req.ActorPassword == "" || len(req.ActorPassword) > MaxPassword {
		return fail(CodeDenied, "administrator password required")
	}
	id, err := h.Auth.Authenticate(req.Actor, req.ActorPassword)
	if err != nil || id.UID < h.MinUID || id.User != req.Actor {
		return fail(CodeDenied, "wrong administrator password")
	}
	if !slices.ContainsFunc(id.Groups, func(g string) bool { return slices.Contains(h.AdminGroups, g) }) {
		return fail(CodeDenied, "not an administrator")
	}
	return nil
}

// CheckPassword validates a new password: length and no control characters
// (chpasswd and smbpasswd read lines).
func CheckPassword(pw string) error {
	if utf8.RuneCountInString(pw) < MinPassword {
		return fail(CodeBadRequest, "the password must be at least %d characters", MinPassword)
	}
	if len(pw) > MaxPassword || !utf8.ValidString(pw) {
		return fail(CodeBadRequest, "the password is too long or not UTF-8")
	}
	for _, r := range pw {
		if unicode.IsControl(r) {
			return fail(CodeBadRequest, "the password must not contain control characters")
		}
	}
	return nil
}

func checkFullName(s string) error {
	if utf8.RuneCountInString(s) > maxFullName || !utf8.ValidString(s) {
		return fail(CodeBadRequest, "the full name is too long")
	}
	for _, r := range s {
		if unicode.IsControl(r) || r == ':' || r == ',' {
			return fail(CodeBadRequest, "the full name must not contain ':' ',' or control characters")
		}
	}
	return nil
}

// reserved names cannot be created even if free.
var reserved = []string{"root", "admin", "administrator", "guest", "nobody", "alpha-admin", "alpha-users", "kinolune", "wheel", "users"}

// --- operations ------------------------------------------------------------------------

func (h *Helper) createUser(ctx context.Context, req Request) error {
	if err := h.verifyAdmin(req); err != nil {
		return err
	}
	if !authhelper.ValidUserName(req.User) || slices.Contains(reserved, req.User) || strings.HasPrefix(req.User, "alpha-") {
		return fail(CodeBadRequest, "invalid user name %q: lower-case latin letters, digits, '-' and '_'", req.User)
	}
	if err := checkFullName(req.FullName); err != nil {
		return err
	}
	if err := CheckPassword(req.Password); err != nil {
		return err
	}
	taken, err := h.existsName(req.User)
	if err != nil {
		return err
	}
	if taken {
		return fail(CodeConflict, "the name %q is taken", req.User)
	}
	groups := samba.UsersGroup
	if req.Admin != nil && *req.Admin {
		groups += ",wheel"
	}
	if _, err := h.Run.Run(ctx, "", "useradd", "-m", "-U", "-s", "/bin/bash", "-G", groups, "-c", req.FullName, "--", req.User); err != nil {
		return err
	}
	if err := h.setPasswords(ctx, req.User, req.Password); err != nil {
		return err
	}
	h.log().Info("user created", "user", req.User, "by", req.Actor, "admin", req.Admin != nil && *req.Admin)
	return h.ensureDirs(ctx, req.User)
}

// setPasswords sets the OS and the Samba password.
func (h *Helper) setPasswords(ctx context.Context, user, pw string) error {
	if _, err := h.Run.Run(ctx, user+":"+pw+"\n", "chpasswd"); err != nil {
		return err
	}
	return h.setSamba(ctx, user, pw)
}

func (h *Helper) setSamba(ctx context.Context, user, pw string) error {
	_, err := h.Run.Run(ctx, pw+"\n"+pw+"\n", "smbpasswd", "-a", "-s", "--", user)
	return err
}

func (h *Helper) setPassword(ctx context.Context, req Request) error {
	if err := h.verifyAdmin(req); err != nil {
		return err
	}
	if err := CheckPassword(req.Password); err != nil {
		return err
	}
	if _, _, _, err := h.human(req.User); err != nil {
		return err
	}
	if err := h.setPasswords(ctx, req.User, req.Password); err != nil {
		return err
	}
	h.log().Info("password changed", "user", req.User, "by", req.Actor)
	return h.ensureDirs(ctx, req.User)
}

func (h *Helper) adminCount(members map[string][]string, accts []account) int {
	n := 0
	for _, a := range accts {
		if inAny(members, h.AdminGroups, a.name) {
			n++
		}
	}
	return n
}

func (h *Helper) setAdmin(ctx context.Context, req Request) error {
	if req.Admin == nil {
		return fail(CodeBadRequest, "admin missing")
	}
	if err := h.verifyAdmin(req); err != nil {
		return err
	}
	_, members, accts, err := h.human(req.User)
	if err != nil {
		return err
	}
	isAdmin := inAny(members, h.AdminGroups, req.User)
	if *req.Admin == isAdmin {
		return nil
	}
	if *req.Admin {
		_, err := h.Run.Run(ctx, "", "usermod", "-a", "-G", "wheel,"+samba.UsersGroup, "--", req.User)
		h.log().Info("administrator added", "user", req.User, "by", req.Actor)
		return err
	}
	if req.User == req.Actor {
		return fail(CodeConflict, "you cannot remove your own administrator rights")
	}
	if h.adminCount(members, accts) <= 1 {
		return fail(CodeConflict, "%s is the last administrator", req.User)
	}
	// Stays a NAS user.
	if !slices.Contains(members[samba.UsersGroup], req.User) {
		if _, err := h.Run.Run(ctx, "", "usermod", "-a", "-G", samba.UsersGroup, "--", req.User); err != nil {
			return err
		}
	}
	for _, g := range h.AdminGroups {
		if slices.Contains(members[g], req.User) {
			if _, err := h.Run.Run(ctx, "", "gpasswd", "-d", req.User, g); err != nil {
				return err
			}
		}
	}
	h.log().Info("administrator removed", "user", req.User, "by", req.Actor)
	return nil
}

func (h *Helper) deleteUser(ctx context.Context, req Request) error {
	if err := h.verifyAdmin(req); err != nil {
		return err
	}
	_, members, accts, err := h.human(req.User)
	if err != nil {
		return err
	}
	if req.User == req.Actor {
		return fail(CodeConflict, "you cannot delete yourself")
	}
	if inAny(members, h.AdminGroups, req.User) && h.adminCount(members, accts) <= 1 {
		return fail(CodeConflict, "%s is the last administrator", req.User)
	}
	if _, err := h.Run.Run(ctx, "", "pdbedit", "-x", "-u", req.User); err != nil {
		h.log().Info("no Samba account to delete", "user", req.User)
	}
	// Files in the shares are kept, out of reach of a future user with the
	// same UID: renamed and handed to root:wheel for the administrators.
	stamp := h.now().UTC().Format("20060102-150405")
	for _, dir := range []string{h.Paths.Users, h.Paths.TimeMachine} {
		p := filepath.Join(dir, req.User)
		if _, err := os.Lstat(p); err != nil {
			continue
		}
		dst := filepath.Join(dir, "deleted-"+req.User+"-"+stamp)
		if _, err := h.Run.Run(ctx, "", "mv", "-T", "--", p, dst); err != nil {
			return err
		}
		if _, err := h.Run.Run(ctx, "", "chown", "-R", "--no-dereference", "root:wheel", "--", dst); err != nil {
			return err
		}
		if _, err := h.Run.Run(ctx, "", "chmod", "-R", "u+rwX,g+rwX,o-rwx", "--", dst); err != nil {
			return err
		}
	}
	if _, err := h.Run.Run(ctx, "", "userdel", "-r", "--", req.User); err != nil {
		return err
	}
	h.log().Info("user deleted", "user", req.User, "by", req.Actor)
	return nil
}

// sambaSync sets the Samba password of a NAS user after the helper checked
// it with PAM itself (called after every successful admin log-in).
func (h *Helper) sambaSync(ctx context.Context, req Request) error {
	if !authhelper.ValidUserName(req.User) || req.Password == "" || len(req.Password) > MaxPassword {
		return fail(CodeDenied, "denied")
	}
	id, err := h.Auth.Authenticate(req.User, req.Password)
	if err != nil || id.UID < h.MinUID || id.User != req.User {
		return fail(CodeDenied, "denied")
	}
	nas := append([]string{}, samba.NASGroups...)
	if !slices.ContainsFunc(id.Groups, func(g string) bool { return slices.Contains(nas, g) }) {
		return nil // not a NAS user: no shares
	}
	if strings.ContainsFunc(req.Password, unicode.IsControl) {
		return fail(CodeBadRequest, "password with control characters")
	}
	if err := h.setSamba(ctx, req.User, req.Password); err != nil {
		return err
	}
	return h.ensureDirs(ctx, req.User)
}

// ensureDirs creates the user's folders in Users and TimeMachine.
func (h *Helper) ensureDirs(ctx context.Context, user string) error {
	for _, dir := range []string{h.Paths.Users, h.Paths.TimeMachine} {
		if _, err := h.Run.Run(ctx, "", "install", "-d", "-m", "0700", "-o", user, "-g", user, "--", filepath.Join(dir, user)); err != nil {
			return err
		}
	}
	return nil
}

func (h *Helper) setSharing(ctx context.Context, s samba.Settings) error {
	old, err := samba.LoadSettings(h.Paths.Settings)
	if err != nil {
		return err
	}
	oldConf, oldErr := os.ReadFile(h.Paths.SambaConf)
	if err := samba.SaveSettings(h.Paths.Settings, s); err != nil {
		return err
	}
	if err := samba.WriteAtomic(h.Paths.SambaConf, []byte(samba.Render(s)), 0o644); err != nil {
		return err
	}
	if _, err := h.Run.Run(ctx, "", "testparm", "-s", "--suppress-prompt"); err != nil {
		// Roll back: never leave smbd with a configuration it refuses.
		_ = samba.SaveSettings(h.Paths.Settings, old)
		if oldErr == nil {
			_ = samba.WriteAtomic(h.Paths.SambaConf, oldConf, 0o644)
		}
		return err
	}
	h.log().Info("sharing settings changed", "media_guest_read", s.MediaGuestRead, "time_machine", s.TimeMachine())
	h.reloadSamba(ctx)
	return nil
}

func (h *Helper) reloadSamba(ctx context.Context) {
	if _, err := h.Run.Run(ctx, "", "smbcontrol", "smbd", "reload-config"); err != nil {
		h.log().Warn("smbd reload (not running?)", "err", err)
	}
}

// RenderBoot is `alpha-admin-priv render-samba`: run as root before smbd at
// every boot. It renders the configuration from the settings, imports the
// installer's Samba password, and creates the folders of every NAS user.
func (h *Helper) RenderBoot(ctx context.Context) error {
	s, err := samba.LoadSettings(h.Paths.Settings)
	if err != nil {
		h.log().Error("settings unreadable, using the defaults", "err", err)
		s = samba.Settings{}
	}
	if err := samba.WriteAtomic(h.Paths.SambaConf, []byte(samba.Render(s)), 0o644); err != nil {
		return err
	}
	if _, err := os.Stat(h.Paths.Import); err == nil {
		if _, err := h.Run.Run(ctx, "", "pdbedit", "-i", "smbpasswd:"+h.Paths.Import, "-e", "tdbsam"); err != nil {
			h.log().Error("import of the installer's Samba password failed", "err", err)
		} else {
			h.log().Info("installer's Samba password imported")
		}
		if err := os.Remove(h.Paths.Import); err != nil {
			return err
		}
	}
	accts, members, err := h.humans()
	if err != nil {
		return err
	}
	for _, a := range accts {
		if inAny(members, samba.NASGroups, a.name) {
			if err := h.ensureDirs(ctx, a.name); err != nil {
				h.log().Warn("folders", "user", a.name, "err", err)
			}
		}
	}
	return nil
}

func (h *Helper) countKeys() int {
	files := []string{h.Paths.RootKeys}
	if m, err := filepath.Glob(h.Paths.HomeGlobKeys); err == nil {
		files = append(files, m...)
	}
	n := 0
	for _, f := range files {
		b, err := os.ReadFile(f)
		if err != nil {
			continue
		}
		for _, l := range strings.Split(string(b), "\n") {
			if l = strings.TrimSpace(l); l != "" && !strings.HasPrefix(l, "#") {
				n++
			}
		}
	}
	return n
}

func (h *Helper) ssh(ctx context.Context) (SSHState, error) {
	st := SSHState{AuthorizedKeys: h.countKeys()}
	out, _ := h.Run.Run(ctx, "", "systemctl", "is-enabled", h.Paths.SSHUnit)
	st.Enabled = strings.TrimSpace(out) == "enabled"
	out, _ = h.Run.Run(ctx, "", "systemctl", "is-active", h.Paths.SSHUnit)
	st.Active = strings.TrimSpace(out) == "active"
	return st, nil
}

func (h *Helper) setSSH(ctx context.Context, on bool) error {
	verb := "disable"
	if on {
		verb = "enable"
	}
	if _, err := h.Run.Run(ctx, "", "systemctl", verb, "--now", h.Paths.SSHUnit); err != nil {
		return err
	}
	h.log().Info("ssh", "enabled", on)
	return nil
}

// --- client ----------------------------------------------------------------------------

// Errors of the client, matching the codes.
var (
	ErrDenied      = errors.New("wrong administrator password")
	ErrBadRequest  = errors.New("bad request")
	ErrNotFound    = errors.New("not found")
	ErrConflict    = errors.New("conflict")
	ErrUnavailable = errors.New("privileged helper unavailable")
)

// Client asks the helper over its unix socket.
type Client struct {
	Socket  string
	Timeout time.Duration
}

// Do sends one request.
func (c *Client) Do(ctx context.Context, req Request) (Response, error) {
	timeout := c.Timeout
	if timeout <= 0 {
		timeout = 90 * time.Second
	}
	ctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	var d net.Dialer
	conn, err := d.DialContext(ctx, "unix", c.Socket)
	if err != nil {
		return Response{}, fmt.Errorf("%w: %v", ErrUnavailable, err)
	}
	defer conn.Close()
	if dl, ok := ctx.Deadline(); ok {
		_ = conn.SetDeadline(dl)
	}
	b, _ := json.Marshal(req)
	if _, err := conn.Write(append(b, '\n')); err != nil {
		return Response{}, fmt.Errorf("%w: %v", ErrUnavailable, err)
	}
	line, err := bufio.NewReaderSize(io.LimitReader(conn, 1<<20), 1<<20).ReadBytes('\n')
	if err != nil && len(line) == 0 {
		return Response{}, fmt.Errorf("%w: %v", ErrUnavailable, err)
	}
	var resp Response
	if err := json.Unmarshal(line, &resp); err != nil {
		return Response{}, fmt.Errorf("%w: bad answer: %v", ErrUnavailable, err)
	}
	if !resp.OK {
		base := ErrUnavailable
		switch resp.Error {
		case CodeDenied:
			base = ErrDenied
		case CodeBadRequest:
			base = ErrBadRequest
		case CodeNotFound:
			base = ErrNotFound
		case CodeConflict:
			base = ErrConflict
		case CodeFailed:
			return resp, fmt.Errorf("system command failed: %s", resp.Message)
		}
		return resp, fmt.Errorf("%w: %s", base, resp.Message)
	}
	return resp, nil
}
