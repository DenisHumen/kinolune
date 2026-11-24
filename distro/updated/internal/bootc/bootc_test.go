// SPDX-License-Identifier: MPL-2.0

package bootc

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestParseStatusStaged(t *testing.T) {
	data, err := os.ReadFile("testdata/status-staged.json")
	if err != nil {
		t.Fatal(err)
	}
	h, err := ParseStatus(data)
	if err != nil {
		t.Fatal(err)
	}
	if got := h.Spec.Image.Image; got != "ghcr.io/denishumen/alpha-os:stable" {
		t.Errorf("spec image = %q", got)
	}
	if got := h.Status.Booted.Image.Version; got != "45.20261113.1" {
		t.Errorf("booted version = %q", got)
	}
	if !h.Status.Staged.DownloadOnly {
		t.Error("staged deployment should be download-only")
	}
	if !strings.HasPrefix(h.Status.Rollback.Digest(), "sha256:1111") {
		t.Errorf("rollback digest = %q", h.Status.Rollback.Digest())
	}
	// The cached update is already staged: nothing new is available.
	if av := h.Available(); av != nil {
		t.Errorf("Available() = %+v, want nil (already staged)", av)
	}
	h.Status.Staged = nil
	if av := h.Available(); av == nil || av.Version != "45.20261120.3" {
		t.Errorf("Available() = %+v, want 45.20261120.3", av)
	}
}

func TestParseStatusErrors(t *testing.T) {
	for name, in := range map[string]string{
		"not json":  "{",
		"wrong":     `{"kind":"Something","status":{"booted":{}}}`,
		"no booted": `{"apiVersion":"org.containers.bootc/v1","kind":"BootcHost","status":{"booted":null}}`,
	} {
		if _, err := ParseStatus([]byte(in)); err == nil {
			t.Errorf("%s: want error", name)
		}
	}
}

func TestAvailableSameAsBooted(t *testing.T) {
	h := &Host{}
	img := &ImageStatus{ImageDigest: "sha256:aa"}
	h.Status.Booted = &BootEntry{Image: img, CachedUpdate: &ImageStatus{ImageDigest: "sha256:aa"}}
	if h.Available() != nil {
		t.Error("cached update equal to booted must not be available")
	}
}

func TestRepositoryAndTag(t *testing.T) {
	for _, tc := range []struct{ ref, repo, tag string }{
		{"ghcr.io/denishumen/alpha-os:stable", "ghcr.io/denishumen/alpha-os", "stable"},
		{"localhost:5000/alpha-os:dev", "localhost:5000/alpha-os", "dev"},
		{"localhost:5000/alpha-os", "localhost:5000/alpha-os", ""},
		{"ghcr.io/o/alpha-os@sha256:abcd", "ghcr.io/o/alpha-os", ""},
		{"ghcr.io/o/alpha-os:beta@sha256:abcd", "ghcr.io/o/alpha-os", ""},
	} {
		if got := Repository(tc.ref); got != tc.repo {
			t.Errorf("Repository(%q) = %q, want %q", tc.ref, got, tc.repo)
		}
		if got := Tag(tc.ref); got != tc.tag {
			t.Errorf("Tag(%q) = %q, want %q", tc.ref, got, tc.tag)
		}
	}
}

// recorder captures the arguments of each call.
type recorder struct{ calls []string }

func (r *recorder) Run(_ context.Context, args ...string) ([]byte, error) {
	r.calls = append(r.calls, strings.Join(args, " "))
	if args[0] == "status" {
		return os.ReadFile("testdata/status-staged.json")
	}
	return nil, nil
}

func TestClientArgs(t *testing.T) {
	r := &recorder{}
	c := Client{R: r}
	ctx := context.Background()
	for _, f := range []func(context.Context) error{c.Check, c.Download, c.Unlock, c.Upgrade, c.Rollback} {
		if err := f(ctx); err != nil {
			t.Fatal(err)
		}
	}
	if err := c.Switch(ctx, "ghcr.io/o/alpha-os:beta"); err != nil {
		t.Fatal(err)
	}
	if _, err := c.Status(ctx); err != nil {
		t.Fatal(err)
	}
	want := []string{
		"upgrade --check", "upgrade --download-only", "upgrade --from-downloaded",
		"upgrade", "rollback", "switch ghcr.io/o/alpha-os:beta", "status --format=json",
	}
	if strings.Join(r.calls, "|") != strings.Join(want, "|") {
		t.Errorf("calls = %q\nwant  %q", r.calls, want)
	}
	if err := c.Switch(ctx, "--transport=dir"); err == nil {
		t.Error("an option must not pass as an image reference")
	}
}

// TestExecRunner runs a fake bootc shell script: arguments arrive intact,
// stdout comes back, a failure carries the exit code and stderr.
func TestExecRunner(t *testing.T) {
	if _, err := os.Stat("/bin/sh"); err != nil {
		t.Skip("no /bin/sh")
	}
	dir := t.TempDir()
	script := filepath.Join(dir, "bootc")
	body := `#!/bin/sh
if [ "$1" = status ]; then echo '{"kind":"BootcHost","status":{"booted":{"image":null}}}'; exit 0; fi
if [ "$1" = upgrade ] && [ "$2" = --check ]; then echo "error: pulling: connection refused" >&2; exit 3; fi
echo "args: $*"
`
	if err := os.WriteFile(script, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
	r := ExecRunner{Path: script}
	out, err := r.Run(context.Background(), "switch", "a b")
	if err != nil || strings.TrimSpace(string(out)) != "args: switch a b" {
		t.Fatalf("out=%q err=%v", out, err)
	}
	_, err = r.Run(context.Background(), "upgrade", "--check")
	var ee *ExecError
	if !errors.As(err, &ee) || ee.ExitCode != 3 || !strings.Contains(ee.Error(), "connection refused") {
		t.Fatalf("err = %v", err)
	}
	if _, err := (Client{R: r}).Status(context.Background()); err != nil {
		t.Fatalf("status via script: %v", err)
	}
	if _, err := (ExecRunner{Path: filepath.Join(dir, "missing")}).Run(context.Background(), "status"); err == nil {
		t.Error("missing binary must fail")
	}
}

// TestExecRunnerStopsGently: a cancelled operation gets SIGTERM, so bootc can
// finish its transaction, not SIGKILL.
func TestExecRunnerStopsGently(t *testing.T) {
	if _, err := os.Stat("/bin/sh"); err != nil {
		t.Skip("no /bin/sh")
	}
	script := filepath.Join(t.TempDir(), "bootc")
	body := "#!/bin/sh\nsleep 30 &\npid=$!\ntrap 'kill $pid; echo got-term >&2; exit 7' TERM\nwait\n"
	if err := os.WriteFile(script, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 300*time.Millisecond)
	defer cancel()
	_, err := ExecRunner{Path: script}.Run(ctx, "upgrade", "--download-only")
	var ee *ExecError
	if !errors.As(err, &ee) || ee.ExitCode != 7 || !strings.Contains(ee.Stderr, "got-term") {
		t.Fatalf("err = %v", err)
	}
}
