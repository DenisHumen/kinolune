// SPDX-License-Identifier: MPL-2.0

package main

import (
	"bytes"
	"context"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
	"github.com/DenisHumen/kinolune/distro/installer/internal/tui"
)

func mustAnswers(t *testing.T, body string) *answers.Answers {
	t.Helper()
	a, err := answers.Load(strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	return a
}

func TestTUINeedsTerminal(t *testing.T) {
	var out bytes.Buffer
	err := runTUI(nil, strings.NewReader(""), &out, &out, testEnv(t, true, false))
	if err == nil || !strings.Contains(err.Error(), "needs a terminal") {
		t.Fatalf("got %v", err)
	}
	err = runTUI(nil, strings.NewReader(""), &out, &out, testEnv(t, false, true))
	if err == nil || !strings.Contains(err.Error(), "as root") {
		t.Fatalf("not root: %v", err)
	}
	err = runTUI([]string{"--answers", writeAnswers(t, `{"version":1,"typo":1}`), "--dry-run"},
		strings.NewReader(""), &out, &out, testEnv(t, false, true))
	if err == nil || !strings.Contains(err.Error(), "typo") {
		t.Fatalf("unknown field: %v", err)
	}
}

func TestTUIEnvDryRunPlan(t *testing.T) {
	e := testEnv(t, false, true)
	te := &tuiEnv{e: e, src: disks.Dir(fixtures + "04-qemu-ci"), dryRun: true, snapshot: true,
		logPath: filepath.Join(t.TempDir(), "install.log")}
	var ui bytes.Buffer
	hash, err := te.Install(context.Background(), mustAnswers(t, nvme), &ui)
	if err != nil || hash != "" {
		t.Fatalf("%q %v\n%s", hash, err, ui.String())
	}
	for _, w := range []string{"Kinolune OS installation plan", "/dev/nvme0n1p3", "Dry run: nothing was written."} {
		if !strings.Contains(ui.String(), w) {
			t.Errorf("plan lacks %q:\n%s", w, ui.String())
		}
	}
	if b, _ := os.ReadFile(te.logPath); !bytes.Equal(b, ui.Bytes()) {
		t.Errorf("log file differs from the screen log")
	}
	if _, err := te.Save(context.Background(), tui.SaveTarget{Device: "/dev/sdd1", FSType: "vfat"}, nil); err == nil {
		t.Error("saved to a device of a snapshot")
	}
}

func TestTUIEnvRefusals(t *testing.T) {
	te := &tuiEnv{e: testEnv(t, true, true), src: disks.Dir(fixtures + "01-live-usb-nas")}
	_, err := te.Install(context.Background(), mustAnswers(t, strings.Replace(nvme, "nvme0n1", "sdd", 1)), io.Discard)
	if err == nil || !strings.Contains(err.Error(), "boots from this disk") {
		t.Errorf("boot media: %v", err)
	}
	te.e.isRoot = func() bool { return false }
	_, err = te.Install(context.Background(), mustAnswers(t, nvme), io.Discard)
	if err == nil || !strings.Contains(err.Error(), "as root") {
		t.Errorf("not root: %v", err)
	}
	bad := mustAnswers(t, nvme)
	bad.Hostname = "Bad_Name"
	if _, err := te.Install(context.Background(), bad, io.Discard); err == nil || !strings.Contains(err.Error(), "hostname") {
		t.Errorf("invalid answers: %v", err)
	}
}

func TestInstallLock(t *testing.T) {
	p := filepath.Join(t.TempDir(), "lock")
	unlock, err := lockInstall(p)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := lockInstall(p); err == nil || !strings.Contains(err.Error(), "another console") {
		t.Errorf("second lock: %v", err)
	}
	unlock()
	unlock2, err := lockInstall(p)
	if err != nil {
		t.Fatalf("after unlock: %v", err)
	}
	unlock2()
}

func TestLoadBaseExample(t *testing.T) {
	a, err := loadBase("../../examples/answers.json")
	if err != nil || a.User.PasswordHash == "" || a.Language != "uk" {
		t.Fatalf("%+v %v", a, err)
	}
	if a, err := loadBase(writeAnswers(t, `{"hostname":"nas"}`)); err != nil || a.Hostname != "nas" {
		t.Fatalf("partial file: %+v %v", a, err)
	}
}
