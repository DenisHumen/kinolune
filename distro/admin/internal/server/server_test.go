// SPDX-License-Identifier: MPL-2.0

package server

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/cookiejar"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/auth"
	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/system"
	"github.com/DenisHumen/kinolune/distro/admin/internal/system/fakebus"
	"github.com/DenisHumen/kinolune/distro/admin/internal/updproxy"
)

// --- fakes ---

type fakeVerifier struct {
	mu    sync.Mutex
	down  bool
	calls int
}

var accounts = map[string]struct {
	pass string
	id   authhelper.Identity
}{
	"anna":  {"correct horse", authhelper.Identity{User: "anna", UID: 1000, Groups: []string{"anna", "wheel"}}},
	"boris": {"battery staple", authhelper.Identity{User: "boris", UID: 1001, Groups: []string{"boris", "alpha-users"}}},
	"carla": {"tape measure", authhelper.Identity{User: "carla", UID: 1002, Groups: []string{"carla", "alpha-admin"}}},
}

func (v *fakeVerifier) count() int {
	v.mu.Lock()
	defer v.mu.Unlock()
	return v.calls
}

func (v *fakeVerifier) setDown() {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.down = true
}

func (v *fakeVerifier) Verify(_ context.Context, user, pass string) (authhelper.Identity, error) {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.calls++
	if v.down {
		return authhelper.Identity{}, authhelper.ErrUnavailable
	}
	a, ok := accounts[user]
	if !ok || a.pass != pass {
		return authhelper.Identity{}, authhelper.ErrDenied
	}
	return a.id, nil
}

// fakeUpdated is alpha-updated on a unix socket: records requests, answers
// like the real API.
type fakeUpdated struct {
	mu   sync.Mutex
	reqs []string
}

func startUpdated(t *testing.T) (string, *fakeUpdated, func()) {
	t.Helper()
	dir, err := os.MkdirTemp("", "upd")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	sock := filepath.Join(dir, "api.sock")
	l, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	f := &fakeUpdated{}
	srv := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		f.mu.Lock()
		f.reqs = append(f.reqs, strings.TrimSpace(r.Method+" "+r.URL.Path+" "+string(body)))
		f.mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		switch r.Method + " " + r.URL.Path {
		case "GET /v1/update":
			_, _ = io.WriteString(w, `{"state":"available","current_version":"0.1.0"}`)
		case "POST /v1/update/check":
			w.WriteHeader(http.StatusAccepted)
			_, _ = io.WriteString(w, `{"operation":"check"}`)
		case "PUT /v1/settings":
			if !json.Valid(body) {
				w.WriteHeader(http.StatusBadRequest)
				_, _ = io.WriteString(w, `{"error":"bad_request"}`)
				return
			}
			_, _ = io.WriteString(w, `{"settings":`+string(body)+`}`)
		case "POST /v1/rollback":
			w.WriteHeader(http.StatusConflict)
			_, _ = io.WriteString(w, `{"error":"conflict","message":"nothing to roll back to"}`)
		default:
			w.WriteHeader(http.StatusNotFound)
		}
	})}
	go func() { _ = srv.Serve(l) }()
	t.Cleanup(func() { srv.Close() })
	return sock, f, func() { srv.Close(); os.Remove(sock) }
}

func (f *fakeUpdated) requests() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return slices.Clone(f.reqs)
}

// --- harness ---

type env struct {
	t       *testing.T
	ts      *httptest.Server
	srv     *Server
	bus     *fakebus.Fake
	ver     *fakeVerifier
	updated *fakeUpdated
	stopUpd func()
	// offset shifts the server clock (atomic: handlers read it concurrently).
	offset atomic.Int64
}

func newEnv(t *testing.T) *env {
	t.Helper()
	fb := fakebus.Start(t)
	bus := &system.Bus{Conn: fb.Client(t), Timeout: 5 * time.Second}
	sock, upd, stop := startUpdated(t)
	proc := t.TempDir()
	for name, body := range map[string]string{
		"uptime":               "3600.50 7000.00\n",
		"loadavg":              "0.50 0.40 0.30 1/200 999\n",
		"meminfo":              "MemTotal:       16000000 kB\nMemFree:  100 kB\nMemAvailable:   12000000 kB\n",
		"cpuinfo":              "processor\t: 0\nmodel name\t: x\n\nprocessor\t: 1\nmodel name\t: x\n",
		"sys/kernel/osrelease": "6.17.0-200.fc45.x86_64\n",
	} {
		p := filepath.Join(proc, name)
		_ = os.MkdirAll(filepath.Dir(p), 0o755)
		if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	ver := &fakeVerifier{}
	srv := &Server{
		Verifier:    ver,
		Sessions:    auth.NewStore(),
		Limiter:     auth.NewLimiter(auth.DefaultLimiterConfig, nil),
		AdminGroups: auth.DefaultAdminGroups,
		Services:    &system.Systemd{Bus: bus, Journalctl: "/nonexistent/journalctl"},
		Network:     &system.NetworkManager{Bus: bus},
		Firewall:    &system.Firewalld{Bus: bus},
		Disks:       &system.UDisks{Bus: bus, SystemUUIDs: []string{fakebus.RootUUID}},
		Power:       &system.Logind{Bus: bus},
		Updates:     updproxy.New(sock),
		Info: Info{
			OSVersion: "0.1.0", TLSFingerprint: "ab:cd", ProcRoot: proc,
			Hostname: func() (string, error) { return "kinolune", nil },
			Install:  &InstallInfo{Language: "uk", UpdateChannel: "stable", InstalledAt: "2026-11-20T10:00:00Z"},
		},
	}
	e := &env{t: t, srv: srv, bus: fb, ver: ver, updated: upd, stopUpd: stop}
	srv.Now = func() time.Time { return time.Now().Add(time.Duration(e.offset.Load())) }
	e.ts = httptest.NewTLSServer(srv)
	t.Cleanup(e.ts.Close)
	return e
}

// client is a browser-like client with a cookie jar.
type client struct {
	e    *env
	hc   *http.Client
	csrf string
}

func (e *env) client() *client {
	jar, _ := cookiejar.New(nil)
	// A copy: httptest hands out one shared *http.Client.
	hc := *e.ts.Client()
	hc.Jar = jar
	return &client{e: e, hc: &hc}
}

type resp struct {
	code   int
	header http.Header
	body   map[string]any
	raw    string
}

func (c *client) do(method, path string, body any, hdr ...string) resp {
	c.e.t.Helper()
	var rd io.Reader
	if body != nil {
		b, _ := json.Marshal(body)
		rd = bytes.NewReader(b)
	}
	req, _ := http.NewRequest(method, c.e.ts.URL+path, rd)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if c.csrf != "" {
		req.Header.Set(CSRFHeader, c.csrf)
	}
	for i := 0; i+1 < len(hdr); i += 2 {
		if hdr[i+1] == "" {
			req.Header.Del(hdr[i])
		} else {
			req.Header.Set(hdr[i], hdr[i+1])
		}
	}
	r, err := c.hc.Do(req)
	if err != nil {
		c.e.t.Fatal(err)
	}
	defer r.Body.Close()
	raw, _ := io.ReadAll(r.Body)
	out := resp{code: r.StatusCode, header: r.Header, raw: string(raw)}
	_ = json.Unmarshal(raw, &out.body)
	return out
}

func (c *client) login(user, pass string) resp {
	c.e.t.Helper()
	r := c.do("POST", "/api/v1/session", map[string]string{"username": user, "password": pass})
	if r.code == http.StatusOK {
		c.csrf, _ = r.body["csrf_token"].(string)
	}
	return r
}

func (e *env) admin() *client {
	c := e.client()
	if r := c.login("anna", "correct horse"); r.code != 200 {
		e.t.Fatalf("admin login: %d %s", r.code, r.raw)
	}
	return c
}

func want(t *testing.T, r resp, code int, errCode string) {
	t.Helper()
	if r.code != code {
		t.Fatalf("status %d, want %d: %s", r.code, code, r.raw)
	}
	if errCode != "" && r.body["error"] != errCode {
		t.Fatalf("error %v, want %s: %s", r.body["error"], errCode, r.raw)
	}
}

// --- tests ---

func TestLoginAndSessionCookie(t *testing.T) {
	e := newEnv(t)
	c := e.client()
	r := c.login("anna", "correct horse")
	want(t, r, 200, "")
	if r.body["role"] != "admin" || c.csrf == "" {
		t.Fatalf("%s", r.raw)
	}
	sc := r.header.Get("Set-Cookie")
	for _, attr := range []string{CookieName + "=", "Path=/", "HttpOnly", "Secure", "SameSite=Strict"} {
		if !strings.Contains(sc, attr) {
			t.Errorf("cookie %q lacks %s", sc, attr)
		}
	}
	if strings.Contains(sc, "Domain=") {
		t.Errorf("__Host- cookie must not have a Domain: %q", sc)
	}
	r = c.do("GET", "/api/v1/session", nil)
	want(t, r, 200, "")
	if r.body["user"] != "anna" || r.body["csrf_token"] != c.csrf {
		t.Fatalf("%s", r.raw)
	}
	if v := e.admin(); v.csrf == c.csrf {
		t.Fatal("CSRF tokens must differ per session")
	}
	if r.header.Get("Cache-Control") != "no-store" || r.header.Get("X-Frame-Options") != "DENY" {
		t.Errorf("security headers: %v", r.header)
	}
}

func TestUnauthenticated(t *testing.T) {
	e := newEnv(t)
	c := e.client()
	for _, p := range []string{"/api/v1/services", "/api/v1/system", "/api/v1/disks", "/api/v1/system/update", "/api/v1/session"} {
		want(t, c.do("GET", p, nil), 401, "unauthorized")
	}
	want(t, c.do("POST", "/api/v1/services/smb.service/restart", nil), 401, "unauthorized")
	// Public endpoints.
	want(t, c.do("GET", "/healthz", nil), 200, "")
	// The web UI is public (it holds no data); the CSP stays strict.
	ui := c.do("GET", "/", nil)
	if ui.code != 200 || !strings.Contains(ui.raw, "/static/js/app.js") || !strings.Contains(ui.header.Get("Content-Security-Policy"), "default-src 'self'") {
		t.Fatalf("index: %d %s", ui.code, ui.header.Get("Content-Security-Policy"))
	}
	if r := c.do("GET", "/static/i18n/uk.json", nil); r.code != 200 || r.body["nav.updates"] != "Оновлення" {
		t.Fatalf("static: %d %.80s", r.code, r.raw)
	}
	want(t, c.do("GET", "/static/missing.js", nil), 404, "")
	r := c.do("GET", "/api/v1/info", nil)
	want(t, r, 200, "")
	if r.body["tls_fingerprint"] != "ab:cd" {
		t.Fatalf("%s", r.raw)
	}
	// A forged cookie does not work.
	c.hc.Jar.SetCookies(mustURL(e.ts.URL), []*http.Cookie{{Name: CookieName, Value: "forged", Path: "/", Secure: true}})
	want(t, c.do("GET", "/api/v1/services", nil), 401, "unauthorized")
}

func TestWrongPasswordAndLockout(t *testing.T) {
	e := newEnv(t)
	c := e.client()
	for i := 0; i < 4; i++ {
		want(t, c.login("anna", "guess"), 401, "invalid_credentials")
	}
	r := c.login("anna", "guess")
	want(t, r, 401, "invalid_credentials")
	if r.header.Get("Retry-After") == "" {
		t.Error("5th failure must announce the lockout")
	}
	calls := e.ver.count()
	// Locked: even the right password is refused without asking the helper.
	r = c.login("anna", "correct horse")
	want(t, r, 429, "locked")
	if r.header.Get("Retry-After") == "" || e.ver.count() != calls {
		t.Fatalf("locked login: %v, helper calls %d→%d", r.header, calls, e.ver.count())
	}
	// Another account from the same address still works.
	want(t, e.client().login("carla", "tape measure"), 200, "")
	// Unknown users look exactly like wrong passwords.
	r = e.client().login("ghost", "x")
	want(t, r, 401, "invalid_credentials")
}

func TestHelperUnavailable(t *testing.T) {
	e := newEnv(t)
	e.ver.setDown()
	want(t, e.client().login("anna", "correct horse"), 503, "unavailable")
}

func TestLoginRefusesCrossOriginAndForms(t *testing.T) {
	e := newEnv(t)
	c := e.client()
	r := c.do("POST", "/api/v1/session", map[string]string{"username": "anna", "password": "correct horse"}, "Origin", "https://evil.example")
	want(t, r, 403, "csrf")
	req, _ := http.NewRequest("POST", e.ts.URL+"/api/v1/session", strings.NewReader("username=anna&password=correct+horse"))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	res, err := c.hc.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	res.Body.Close()
	if res.StatusCode != http.StatusUnsupportedMediaType {
		t.Fatalf("form login: %d", res.StatusCode)
	}
	want(t, c.do("POST", "/api/v1/session", map[string]any{"username": "anna", "password": "correct horse", "extra": 1}), 400, "bad_request")
}

func TestCSRF(t *testing.T) {
	e := newEnv(t)
	c := e.admin()
	want(t, c.do("POST", "/api/v1/services/smb.service/restart", nil, CSRFHeader, ""), 403, "csrf")
	want(t, c.do("POST", "/api/v1/services/smb.service/restart", nil, CSRFHeader, "wrong"), 403, "csrf")
	want(t, c.do("POST", "/api/v1/services/smb.service/restart", nil, "Origin", "https://evil.example"), 403, "csrf")
	want(t, c.do("POST", "/api/v1/services/smb.service/restart", nil, "Origin", "null"), 403, "csrf")
	if got := e.bus.Calls(); len(got) != 0 {
		t.Fatalf("refused requests reached D-Bus: %q", got)
	}
	host := strings.TrimPrefix(e.ts.URL, "https://")
	r := c.do("POST", "/api/v1/services/smb.service/restart", nil, "Origin", "https://"+host)
	want(t, r, 202, "")
	if got := e.bus.Calls(); !slices.Equal(got, []string{"RestartUnit smb.service replace"}) {
		t.Fatalf("calls %q", got)
	}
}

func TestNonAdminCannotLogIn(t *testing.T) {
	e := newEnv(t)
	c := e.client()
	r := c.login("boris", "battery staple")
	want(t, r, 403, "not_admin")
	if r.body["message"] != NotAdminMessage || NotAdminMessage != "Нет прав администратора" {
		t.Fatalf("%s", r.raw)
	}
	if strings.Contains(r.header.Get("Set-Cookie"), CookieName) || e.srv.Sessions.Len() != 0 {
		t.Fatal("a non-admin got a session")
	}
	for _, p := range []string{"/api/v1/services", "/api/v1/disks", "/api/v1/system/update"} {
		want(t, c.do("GET", p, nil), 401, "unauthorized")
	}
	// A right password is not a failure: no lockout after many tries.
	for i := 0; i < 6; i++ {
		want(t, e.client().login("boris", "battery staple"), 403, "not_admin")
	}
	if got := e.bus.Calls(); len(got) != 0 {
		t.Fatalf("non-admin reached D-Bus: %q", got)
	}
	if got := e.updated.requests(); len(got) != 0 {
		t.Fatalf("non-admin reached alpha-updated: %q", got)
	}
}

// A session that is not an administrator's (none are created today) is
// still refused everywhere.
func TestNonAdminSessionRefused(t *testing.T) {
	e := newEnv(t)
	token, sess := e.srv.Sessions.Create("boris", 1001, auth.Role("guest"), "127.0.0.1")
	c := e.client()
	c.csrf = sess.CSRF
	c.hc.Jar.SetCookies(mustURL(e.ts.URL), []*http.Cookie{{Name: CookieName, Value: token, Path: "/", Secure: true}})
	want(t, c.do("GET", "/api/v1/services", nil), 403, "forbidden")
	want(t, c.do("GET", "/api/v1/system/update", nil), 403, "forbidden")
	want(t, c.do("POST", "/api/v1/services/smb.service/restart", nil), 403, "forbidden")
}

func TestLogout(t *testing.T) {
	e := newEnv(t)
	c := e.admin()
	want(t, c.do("DELETE", "/api/v1/session", nil, CSRFHeader, ""), 403, "csrf")
	want(t, c.do("DELETE", "/api/v1/session", nil), 204, "")
	want(t, c.do("GET", "/api/v1/session", nil), 401, "unauthorized")
	if e.srv.Sessions.Len() != 0 {
		t.Fatal("session not removed")
	}
}

func TestServicesAPI(t *testing.T) {
	e := newEnv(t)
	c := e.admin()
	r := c.do("GET", "/api/v1/services/alpha-media.service", nil)
	want(t, r, 200, "")
	if r.body["active_state"] != "active" || r.body["title"] != "Медиасервер" {
		t.Fatalf("%s", r.raw)
	}
	want(t, c.do("GET", "/api/v1/services/getty@tty1.service", nil), 404, "not_found")
	want(t, c.do("POST", "/api/v1/services/sshd.service/restart", nil), 403, "forbidden")
	e.bus.Deny("RestartUnit")
	want(t, c.do("POST", "/api/v1/services/alpha-media.service/restart", nil), 403, "forbidden")
	want(t, c.do("GET", "/api/v1/services/alpha-media.service/journal?lines=0", nil), 400, "bad_request")
	// journalctl is missing in the test: reported as a system error.
	want(t, c.do("GET", "/api/v1/services/alpha-media.service/journal?lines=5", nil), 502, "system")
}

func TestNetworkFirewallDisksAPI(t *testing.T) {
	e := newEnv(t)
	c := e.admin()
	r := c.do("GET", "/api/v1/network", nil)
	want(t, r, 200, "")
	if r.body["state"] != "connected-global" {
		t.Fatalf("%s", r.raw)
	}
	want(t, c.do("POST", "/api/v1/network/connections/"+fakebus.WifiUUID+"/up", nil), 202, "")
	want(t, c.do("POST", "/api/v1/network/connections/not-a-uuid/up", nil), 400, "bad_request")
	want(t, c.do("POST", "/api/v1/network/connections/00000000-0000-0000-0000-000000000000/down", nil), 404, "not_found")
	want(t, c.do("PUT", "/api/v1/network/wireless", map[string]any{}), 400, "bad_request")
	want(t, c.do("PUT", "/api/v1/network/wireless", map[string]bool{"enabled": false}), 200, "")

	r = c.do("GET", "/api/v1/firewall", nil)
	want(t, r, 200, "")
	if r.body["default_zone"] != "home" {
		t.Fatalf("%s", r.raw)
	}
	want(t, c.do("PUT", "/api/v1/firewall/openings/cockpit", map[string]bool{"open": true}), 200, "")
	want(t, c.do("PUT", "/api/v1/firewall/openings/https", map[string]bool{"open": false}), 404, "not_found")
	if s, _, p, _ := e.bus.Zone("home"); !slices.Contains(s, "cockpit") || !slices.Contains(p, "cockpit") {
		t.Fatalf("cockpit not opened: %v %v", s, p)
	}

	r = c.do("GET", "/api/v1/disks", nil)
	want(t, r, 200, "")
	if n := len(r.body["drives"].([]any)); n != 3 {
		t.Fatalf("%d drives", n)
	}
	r = c.do("POST", "/api/v1/disks/sda1/mount", nil)
	want(t, r, 200, "")
	if r.body["mount_point"] != "/run/media/alpha-admin/media" {
		t.Fatalf("%s", r.raw)
	}
	want(t, c.do("POST", "/api/v1/disks/nvme0n1p2/unmount", nil), 403, "forbidden")
	want(t, c.do("POST", "/api/v1/disks/..%2fsda/mount", nil), 400, "bad_request")
}

func TestPowerConfirmation(t *testing.T) {
	e := newEnv(t)
	c := e.admin()
	r := c.do("POST", "/api/v1/power/reboot", nil)
	want(t, r, 409, "confirmation_required")
	token, _ := r.body["confirm_token"].(string)
	inh, _ := r.body["inhibitors"].([]any)
	if token == "" || len(inh) != 1 {
		t.Fatalf("%s", r.raw)
	}
	if len(e.bus.Calls()) != 0 {
		t.Fatal("rebooted without confirmation")
	}
	// The token is bound to the action and the session.
	want(t, c.do("POST", "/api/v1/power/poweroff", map[string]string{"confirm": token}), 409, "confirmation_invalid")
	r = c.do("POST", "/api/v1/power/reboot", nil)
	token = r.body["confirm_token"].(string)
	other := e.admin()
	want(t, other.do("POST", "/api/v1/power/reboot", map[string]string{"confirm": token}), 409, "confirmation_invalid")
	// Single use: the failed attempt above consumed it.
	want(t, c.do("POST", "/api/v1/power/reboot", map[string]string{"confirm": token}), 409, "confirmation_invalid")
	r = c.do("POST", "/api/v1/power/reboot", nil)
	token = r.body["confirm_token"].(string)
	want(t, c.do("POST", "/api/v1/power/reboot", map[string]string{"confirm": token}), 202, "")
	if got := e.bus.Calls(); !slices.Equal(got, []string{"Reboot false"}) {
		t.Fatalf("calls %q", got)
	}
	// Expiry.
	r = c.do("POST", "/api/v1/power/poweroff", nil)
	token = r.body["confirm_token"].(string)
	e.offset.Store(int64(2 * time.Minute))
	want(t, c.do("POST", "/api/v1/power/poweroff", map[string]string{"confirm": token}), 409, "confirmation_invalid")
	want(t, c.do("POST", "/api/v1/power/suspend", nil), 404, "not_found")
}

func TestUpdateProxy(t *testing.T) {
	e := newEnv(t)
	c := e.admin()
	r := c.do("GET", "/api/v1/system/update", nil)
	want(t, r, 200, "")
	if r.body["state"] != "available" {
		t.Fatalf("%s", r.raw)
	}
	want(t, c.do("POST", "/api/v1/system/update/check", nil), 202, "")
	r = c.do("PUT", "/api/v1/system/update/settings", map[string]string{"window": "02:00-04:00"})
	want(t, r, 200, "")
	// alpha-updated's own errors pass through unchanged.
	want(t, c.do("POST", "/api/v1/system/update/rollback", map[string]bool{"reboot": true}), 409, "conflict")
	want(t, c.do("GET", "/api/v1/system/update/../../../v1/status", nil), 404, "")
	want(t, c.do("GET", "/api/v1/system/update/nope", nil), 404, "not_found")
	want(t, c.do("PATCH", "/api/v1/system/update/settings", nil), 405, "method_not_allowed")
	want(t, c.do("POST", "/api/v1/system/update/check", nil, CSRFHeader, ""), 403, "csrf")
	got := e.updated.requests()
	exp := []string{
		"GET /v1/update",
		"POST /v1/update/check",
		`PUT /v1/settings {"window":"02:00-04:00"}`,
		`POST /v1/rollback {"reboot":true}`,
	}
	if !slices.Equal(got, exp) {
		t.Fatalf("forwarded %q\nwant %q", got, exp)
	}
	// Agent down.
	e.stopUpd()
	want(t, c.do("GET", "/api/v1/system/update/status", nil), 502, "update_agent_unavailable")
}

func TestSystemInfo(t *testing.T) {
	e := newEnv(t)
	r := e.admin().do("GET", "/api/v1/system", nil)
	want(t, r, 200, "")
	b := r.body
	if b["hostname"] != "kinolune" || b["os_version"] != "0.1.0" || b["uptime_seconds"] != float64(3600) ||
		b["cpus"] != float64(2) || b["memory_available"] != float64(12000000*1024) || b["kernel"] != "6.17.0-200.fc45.x86_64" {
		t.Fatalf("%s", r.raw)
	}
	if inst := b["install"].(map[string]any); inst["language"] != "uk" {
		t.Fatalf("%s", r.raw)
	}
}

func TestOpenAPICoversRoutes(t *testing.T) {
	doc := string(OpenAPI())
	for _, p := range []string{
		"/api/v1/info:", "/api/v1/session:", "/api/v1/system:", "/api/v1/services:", "/api/v1/services/{unit}:",
		"/api/v1/services/{unit}/restart:", "/api/v1/services/{unit}/journal:", "/api/v1/network:",
		"/api/v1/network/connections:", "/api/v1/network/connections/{uuid}/up:", "/api/v1/network/connections/{uuid}/down:",
		"/api/v1/network/wireless:", "/api/v1/firewall:", "/api/v1/firewall/openings/{name}:", "/api/v1/disks:",
		"/api/v1/disks/{device}/mount:", "/api/v1/disks/{device}/unmount:", "/api/v1/power:", "/api/v1/power/{action}:",
		"/api/v1/users:", "/api/v1/users/{name}:", "/api/v1/users/{name}/password:", "/api/v1/users/{name}/admin:",
		"/api/v1/sharing:", "/api/v1/ssh:",
	} {
		if !strings.Contains(doc, "\n  "+p+"\n") {
			t.Errorf("openapi.yaml lacks %s", p)
		}
	}
	for _, rt := range updproxy.Routes {
		if !strings.Contains(doc, "\n  "+updatePrefix+rt.Path+":\n") {
			t.Errorf("openapi.yaml lacks %s%s", updatePrefix, rt.Path)
		}
	}
	e := newEnv(t)
	r := e.client().do("GET", "/api/v1/openapi.yaml", nil)
	if r.code != 200 || !strings.HasPrefix(r.raw, "# SPDX") {
		t.Fatalf("%d", r.code)
	}
}

func TestInstallInfo(t *testing.T) {
	p := filepath.Join(t.TempDir(), "install.json")
	if info, err := LoadInstallInfo(p); info != nil || err != nil {
		t.Fatalf("missing file: %v %v", info, err)
	}
	_ = os.WriteFile(p, []byte(`{"schema":1,"hostname":"nas","filesystems":{"root":"R","boot":"B","media":"M"},
		"data_pool":{"name":"media","mountpoint":"/var/srv/data/media","uuid":"M"},"extra_field":true}`), 0o644)
	info, err := LoadInstallInfo(p)
	if err != nil {
		t.Fatal(err)
	}
	got := info.SystemUUIDs()
	slices.Sort(got)
	if !slices.Equal(got, []string{"B", "R"}) {
		t.Fatalf("%v", got)
	}
}

func mustURL(s string) *url.URL { u, _ := url.Parse(s); return u }
