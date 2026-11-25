// SPDX-License-Identifier: MPL-2.0

package main

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
	"github.com/DenisHumen/kinolune/distro/installer/internal/install"
)

const fixtures = "../../internal/disks/testdata/"

// noRunner fails the test on any command: the CLI paths tested here must
// never reach the disk.
type noRunner struct{ t *testing.T }

func (r noRunner) Run(_ context.Context, c install.Cmd) ([]byte, error) {
	r.t.Errorf("unexpected command: %s", c)
	return nil, nil
}
func (noRunner) LookPath(string) bool { return true }

// testEnv serves a recorded snapshot as the "live" machine.
func testEnv(t *testing.T, root, tty bool) env {
	return env{
		runner:  noRunner{t},
		resolve: func(p string) (string, error) { return p, nil },
		isTTY:   func() bool { return tty },
		isRoot:  func() bool { return root },
		live:    disks.Dir(fixtures + "04-qemu-ci"),
	}
}

func writeAnswers(t *testing.T, body string) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "answers.json")
	if err := os.WriteFile(p, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	return p
}

const nvme = `{"version":1,"disk":{"path":"/dev/nvme0n1"},"user":{"name":"admin","password":"x"}}`

func TestDryRunFromSnapshot(t *testing.T) {
	var out bytes.Buffer
	err := runInstall(context.Background(),
		[]string{"--answers", writeAnswers(t, nvme), "--dry-run", "--from", fixtures + "04-qemu-ci"},
		nil, &out, &out, testEnv(t, false, false))
	if err != nil {
		t.Fatalf("%v\n%s", err, out.String())
	}
	for _, w := range []string{"QEMU NVMe Ctrl — /dev/nvme0n1", "/dev/nvme0n1p3", "Dry run: nothing was written."} {
		if !strings.Contains(out.String(), w) {
			t.Errorf("output lacks %q:\n%s", w, out.String())
		}
	}
}

func TestInstallRefusals(t *testing.T) {
	ans := writeAnswers(t, nvme)
	cases := map[string]struct {
		args  []string
		root  bool
		tty   bool
		stdin string
		want  string
	}{
		"no --yes, no tty": {[]string{"--answers", ans}, true, false, "", "without confirmation: add --yes"},
		"wrong name typed": {[]string{"--answers", ans}, true, true, "sda\n", "cancelled"},
		"empty answer":     {[]string{"--answers", ans}, true, true, "", "cancelled"},
		"not root":         {[]string{"--answer", ans, "--yes"}, false, false, "", "as root"},
		"from without dry": {[]string{"--answers", ans, "--from", "x"}, true, false, "", "only with --dry-run"},
		"no answers":       {[]string{"--yes"}, true, false, "", "needs --answers"},
		"stray argument":   {[]string{"--answers", ans, "/dev/sda"}, true, false, "", "unexpected argument"},
		"bad answers":      {[]string{"--answers", writeAnswers(t, `{"version":1}`), "--yes"}, true, false, "", "disk.path"},
		"optical refused":  {[]string{"--answers", writeAnswers(t, strings.Replace(nvme, "nvme0n1", "sr0", 1)), "--yes"}, true, false, "", "optical"},
	}
	for name, c := range cases {
		t.Run(name, func(t *testing.T) {
			var out bytes.Buffer
			err := runInstall(context.Background(), c.args, strings.NewReader(c.stdin), &out, &out, testEnv(t, c.root, c.tty))
			if err == nil || !strings.Contains(err.Error(), c.want) {
				t.Fatalf("got %v, want %q\n%s", err, c.want, out.String())
			}
		})
	}
}

func TestLiveBootMediaRefused(t *testing.T) {
	e := testEnv(t, true, false)
	e.live = disks.Dir(fixtures + "01-live-usb-nas")
	e.resolve = func(string) (string, error) { return "/dev/sdd", nil }
	var out bytes.Buffer
	err := runInstall(context.Background(),
		[]string{"--answers", writeAnswers(t, strings.Replace(nvme, "/dev/nvme0n1", "/dev/disk/by-label/ALPHA-OS-INSTALL", 1)), "--yes"},
		nil, &out, &out, e)
	if err == nil || !strings.Contains(err.Error(), "boots from this disk") {
		t.Fatalf("got %v", err)
	}
}
