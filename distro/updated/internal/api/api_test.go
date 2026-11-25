// SPDX-License-Identifier: MPL-2.0

package api

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/DenisHumen/kinolune/distro/updated/internal/agent"
	"github.com/DenisHumen/kinolune/distro/updated/internal/bootc"
	"github.com/DenisHumen/kinolune/distro/updated/internal/fakebootc"
)

const ref = "ghcr.io/denishumen/alpha-os:stable"

type syncBuf struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (s *syncBuf) Write(p []byte) (int, error) { s.mu.Lock(); defer s.mu.Unlock(); return s.b.Write(p) }
func (s *syncBuf) String() string              { s.mu.Lock(); defer s.mu.Unlock(); return s.b.String() }

type fixture struct {
	fb      *fakebootc.Fake
	a       *agent.Agent
	c       *http.Client
	logs    *syncBuf
	reboots int
	sock    string
}

// start serves the real agent on a fake bootc over a real unix socket.
func start(t *testing.T) *fixture {
	t.Helper()
	// Short path: unix socket paths are limited to ~108 bytes.
	dir, err := os.MkdirTemp("", "au")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	f := &fixture{fb: fakebootc.New(ref, fakebootc.Image{Digest: "sha256:v1", Version: "45.1"}), logs: &syncBuf{}}
	log := slog.New(slog.NewJSONHandler(f.logs, nil))
	f.a = agent.New(agent.Config{StateDir: filepath.Join(dir, "state"), Logger: log}, bootc.Client{R: f.fb}, agent.NoVeto{},
		agent.RebootFunc(func(context.Context) error { f.reboots++; f.fb.Boot(); return nil }))
	f.a.Start(context.Background())
	f.sock = filepath.Join(dir, "api.sock")
	ln, err := net.Listen("unix", f.sock)
	if err != nil {
		t.Fatal(err)
	}
	srv := (&Server{A: f.a, Log: log}).NewHTTPServer()
	go srv.Serve(ln)
	t.Cleanup(func() { srv.Close(); f.a.Wait() })
	f.c = &http.Client{
		Timeout: 5 * time.Second,
		Transport: &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, "unix", f.sock)
		}},
	}
	return f
}

func (f *fixture) do(t *testing.T, method, path, body string) (int, map[string]any) {
	t.Helper()
	var r io.Reader
	if body != "" {
		r = strings.NewReader(body)
	}
	req, _ := http.NewRequest(method, "http://alpha-updated"+path, r)
	resp, err := f.c.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if ct := resp.Header.Get("Content-Type"); resp.StatusCode != 404 && resp.StatusCode != 405 && ct != "application/json" {
		t.Errorf("%s %s: content type %q", method, path, ct)
	}
	var m map[string]any
	data, _ := io.ReadAll(resp.Body)
	_ = json.Unmarshal(data, &m)
	return resp.StatusCode, m
}

func TestHealthAndStatus(t *testing.T) {
	f := start(t)
	if code, m := f.do(t, "GET", "/healthz", ""); code != 200 || m["status"] != "ok" {
		t.Fatalf("healthz %d %v", code, m)
	}
	code, m := f.do(t, "GET", "/v1/status", "")
	if code != 200 {
		t.Fatalf("status %d", code)
	}
	booted := m["booted"].(map[string]any)
	if booted["version"] != "45.1" || m["tracking"] != ref {
		t.Fatalf("status = %v", m)
	}
	if m["update"].(map[string]any)["state"] != agent.PhaseUpToDate {
		t.Fatalf("update = %v", m["update"])
	}
	settings := m["settings"].(map[string]any)
	if settings["channel"] != "stable" || settings["auto"] != "install" || settings["window"] != "03:00-05:00" {
		t.Fatalf("settings = %v", settings)
	}
}

func TestCheckDownloadApplyOverSocket(t *testing.T) {
	f := start(t)
	f.fb.Publish(ref, fakebootc.Image{Digest: "sha256:v2", Version: "45.2"})

	code, m := f.do(t, "POST", "/v1/update/check", "")
	if code != 202 || m["operation"].(map[string]any)["kind"] != "check" {
		t.Fatalf("check %d %v", code, m)
	}
	f.a.Wait()
	code, m = f.do(t, "GET", "/v1/update", "")
	if code != 200 || m["state"] != agent.PhaseAvailable || m["available"].(map[string]any)["version"] != "45.2" {
		t.Fatalf("update %d %v", code, m)
	}

	if code, _ := f.do(t, "POST", "/v1/update/download", ""); code != 202 {
		t.Fatalf("download %d", code)
	}
	f.a.Wait()
	if _, m := f.do(t, "GET", "/v1/update", ""); m["state"] != agent.PhaseDownloaded {
		t.Fatalf("after download %v", m)
	}

	if code, m := f.do(t, "POST", "/v1/update/apply", `{"when":"maintenance"}`); code != 202 || m["pending_apply"] != true {
		t.Fatalf("maintenance %d %v", code, m)
	}
	if code, m := f.do(t, "DELETE", "/v1/update/apply", ""); code != 200 || m["pending_apply"] != false {
		t.Fatalf("cancel %d %v", code, m)
	}
	if code, m := f.do(t, "POST", "/v1/update/apply", `{"when":"later"}`); code != 400 || m["code"] != "bad_request" {
		t.Fatalf("bad when %d %v", code, m)
	}
	if code, _ := f.do(t, "POST", "/v1/update/apply", `{"when":"now"}`); code != 202 {
		t.Fatalf("apply %d", code)
	}
	f.a.Wait()
	if f.reboots != 1 || f.fb.Booted() != "sha256:v2" {
		t.Fatalf("reboots %d booted %s", f.reboots, f.fb.Booted())
	}
	// Mutations are audited with the peer's credentials.
	logs := f.logs.String()
	if !strings.Contains(logs, `"path":"/v1/update/apply"`) {
		t.Fatalf("no audit log: %s", logs)
	}
	if !strings.Contains(logs, `"uid":`) && isLinux() {
		t.Fatalf("no peer credentials in audit log: %s", logs)
	}
}

func TestRollbackOverSocket(t *testing.T) {
	f := start(t)
	if code, _ := f.do(t, "POST", "/v1/rollback", `{"reboot":false}`); code != 202 {
		t.Fatalf("rollback %d", code)
	}
	f.a.Wait()
	_, m := f.do(t, "GET", "/v1/update", "")
	if le, _ := m["last_error"].(map[string]any); le == nil || le["op"] != "rollback" {
		t.Fatalf("expected a rollback error without a previous deployment: %v", m)
	}
}

func TestBusyIs409(t *testing.T) {
	f := start(t)
	block := make(chan struct{})
	f.fb.Block = block
	if code, _ := f.do(t, "POST", "/v1/update/check", ""); code != 202 {
		t.Fatalf("check %d", code)
	}
	code, m := f.do(t, "POST", "/v1/update/download", "")
	if code != 409 || m["code"] != "busy" {
		t.Fatalf("busy %d %v", code, m)
	}
	if code, m := f.do(t, "PUT", "/v1/settings", `{"channel":"beta"}`); code != 409 {
		t.Fatalf("switch while busy %d %v", code, m)
	}
	close(block)
	f.a.Wait()
}

func TestSettingsOverSocket(t *testing.T) {
	f := start(t)
	for _, body := range []string{`{"channel":"nightly"}`, `{"window":"25:00-01:00"}`, `{"auto":"sometimes"}`, `{"colour":"red"}`, `{`} {
		if code, m := f.do(t, "PUT", "/v1/settings", body); code != 400 {
			t.Errorf("PUT %s: %d %v", body, code, m)
		}
	}
	code, m := f.do(t, "PUT", "/v1/settings", `{"auto":false,"window":"23:30-01:00"}`)
	if code != 200 {
		t.Fatalf("put %d %v", code, m)
	}
	if s := m["settings"].(map[string]any); s["auto"] != "off" || s["window"] != "23:30-01:00" || s["channel"] != "stable" {
		t.Fatalf("settings = %v", s)
	}
	if _, m := f.do(t, "GET", "/v1/settings", ""); m["auto"] != "off" {
		t.Fatalf("get = %v", m)
	}
	// Channel change starts a switch.
	f.fb.Publish("ghcr.io/denishumen/alpha-os:beta", fakebootc.Image{Digest: "sha256:b1", Version: "45.3"})
	code, m = f.do(t, "PUT", "/v1/settings", `{"channel":"beta"}`)
	if code != 200 || m["operation"].(map[string]any)["kind"] != "switch" {
		t.Fatalf("switch %d %v", code, m)
	}
	f.a.Wait()
	if _, m := f.do(t, "GET", "/v1/status", ""); m["tracking"] != "ghcr.io/denishumen/alpha-os:beta" {
		t.Fatalf("tracking = %v", m["tracking"])
	}
}

func TestMethodsAndBodies(t *testing.T) {
	f := start(t)
	if code, _ := f.do(t, "GET", "/v1/rollback", ""); code != 405 {
		t.Errorf("GET rollback = %d", code)
	}
	if code, _ := f.do(t, "GET", "/v1/nope", ""); code != 404 {
		t.Errorf("unknown = %d", code)
	}
	big := `{"when":"` + strings.Repeat("x", maxBody) + `"}`
	if code, _ := f.do(t, "POST", "/v1/update/apply", big); code != 400 {
		t.Errorf("oversized body = %d", code)
	}
}
