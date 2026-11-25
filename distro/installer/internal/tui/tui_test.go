// SPDX-License-Identifier: MPL-2.0

package tui

import (
	"bytes"
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/muesli/termenv"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
	"github.com/DenisHumen/kinolune/distro/installer/internal/install"
)

var update = flag.Bool("update", false, "rewrite the golden screens in testdata/golden")

const fixtures = "../disks/testdata/"

func TestMain(m *testing.M) {
	// Golden screens are plain text: no colours, as on a serial console.
	lipgloss.SetColorProfile(termenv.Ascii)
	os.Exit(m.Run())
}

// fakeEnv serves a recorded disk snapshot and records what the TUI asks for.
type fakeEnv struct {
	t       *testing.T
	fixture string
	mutate  func([]disks.Disk) []disks.Disk
	ifaces  []Iface

	mu        sync.Mutex
	installed []*answers.Answers
	installFn func(a *answers.Answers, log io.Writer) (string, error)
	saved     [][]byte
}

func (f *fakeEnv) Disks() ([]disks.Disk, error) {
	list, err := disks.Probe(disks.Dir(fixtures+f.fixture), disks.Options{})
	if f.mutate != nil {
		list = f.mutate(list)
	}
	return list, err
}

func (f *fakeEnv) Interfaces() []Iface { return f.ifaces }

func (f *fakeEnv) Install(_ context.Context, a *answers.Answers, log io.Writer) (string, error) {
	f.mu.Lock()
	f.installed = append(f.installed, a)
	f.mu.Unlock()
	if f.installFn != nil {
		return f.installFn(a, log)
	}
	fmt.Fprintln(log, "==> Partitioning /dev/nvme0n1 (Samsung 990 PRO 1TB, 1.0 TB)")
	fmt.Fprintln(log, "+ sfdisk --wipe always /dev/nvme0n1 <<EOF ...")
	fmt.Fprint(log, "Copying blob 1/3\rCopying blob 2/3\rCopying blob 3/3\n")
	fmt.Fprintln(log, "==> Installed Kinolune OS to /dev/nvme0n1; it tracks ghcr.io/denishumen/alpha-os:stable")
	return "$y$j9T$7nVyZ3WBHdkfQ8WaD9Xgd.$Dn3Cl9mI6/ybf8z7FNnUW2ePrCVc6kB8BDu7Rbq3wf4", nil
}

func (f *fakeEnv) SaveTargets(exclude string) []SaveTarget {
	list, _ := disks.Probe(disks.Dir(fixtures+"07-usb-mix"), disks.Options{})
	return FindSaveTargets(list, exclude, "/root/kinolune-answers.json")
}

func (f *fakeEnv) Save(_ context.Context, t SaveTarget, data []byte) (string, error) {
	f.saved = append(f.saved, data)
	return t.Device + ":/" + AnswerFileName, nil
}

var testIfaces = []Iface{{Name: "enp1s0", MAC: "52:54:00:12:34:56", Carrier: true}, {Name: "enp2s0", MAC: "52:54:00:ab:cd:ef"}}

// harness drives the model like the Bubble Tea runtime: every command is
// executed and its message fed back, so the install goroutine runs to the end.
type harness struct {
	t    *testing.T
	m    *Model
	env  *fakeEnv
	quit bool
}

var t0 = time.Date(2026, 11, 26, 10, 0, 0, 0, time.UTC)

func newHarness(t *testing.T, fixture string, opt Options) *harness {
	env := &fakeEnv{t: t, fixture: fixture, ifaces: testIfaces}
	clock := t0
	if opt.Now == nil {
		opt.Now = func() time.Time { clock = clock.Add(65 * time.Second); return clock }
	}
	if opt.Version == "" {
		opt.Version = "0.1.0"
	}
	h := &harness{t: t, env: env}
	h.m = New(env, opt)
	h.send(tea.WindowSizeMsg{Width: 80, Height: 24})
	return h
}

func (h *harness) send(msg tea.Msg) {
	_, cmd := h.m.Update(msg)
	h.run(cmd)
}

func (h *harness) run(cmd tea.Cmd) {
	if cmd == nil {
		return
	}
	switch msg := cmd().(type) {
	case nil:
	case tea.BatchMsg:
		for _, c := range msg {
			h.run(c)
		}
	case tea.QuitMsg:
		h.quit = true
	default:
		h.send(msg)
	}
}

var keyNames = map[string]tea.KeyType{
	"enter": tea.KeyEnter, "esc": tea.KeyEsc, "up": tea.KeyUp, "down": tea.KeyDown, "left": tea.KeyLeft,
	"right": tea.KeyRight, "tab": tea.KeyTab, "shift+tab": tea.KeyShiftTab, "space": tea.KeySpace,
	"ctrl+c": tea.KeyCtrlC, "backspace": tea.KeyBackspace, "ctrl+u": tea.KeyCtrlU,
}

// keys presses named keys; "=text" types text.
func (h *harness) keys(ks ...string) {
	h.t.Helper()
	for _, k := range ks {
		if strings.HasPrefix(k, "=") {
			h.send(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune(k[1:])})
			continue
		}
		typ, ok := keyNames[k]
		if !ok {
			h.t.Fatalf("unknown key %q", k)
		}
		msg := tea.KeyMsg{Type: typ}
		if typ == tea.KeySpace {
			msg.Runes = []rune{' '}
		}
		h.send(msg)
	}
}

// typeField replaces the value of the focused text field.
func (h *harness) typeField(s string) {
	h.keys("ctrl+u")
	if s != "" {
		h.keys("=" + s)
	}
}

func (h *harness) want(s screen) {
	h.t.Helper()
	if h.m.scr != s {
		h.t.Fatalf("screen %d, want %d; message %q\n%s", h.m.scr, s, h.m.msg, h.m.View())
	}
}

func (h *harness) golden(name string) {
	h.t.Helper()
	view := h.m.View()
	lines := strings.Split(view, "\n")
	if len(lines) != 24 {
		h.t.Errorf("%s: %d lines, want 24", name, len(lines))
	}
	for i, l := range lines {
		if w := lipgloss.Width(l); w > 80 {
			h.t.Errorf("%s: line %d is %d columns wide: %q", name, i+1, w, l)
		}
		if strings.ContainsAny(l, "…←↑→↓─│—") {
			h.t.Errorf("%s: line %d has a glyph the Linux console font may lack: %q", name, i+1, l)
		}
	}
	// Trailing spaces are noise in the golden files.
	for i, l := range lines {
		lines[i] = strings.TrimRight(l, " ")
	}
	got := strings.Join(lines, "\n") + "\n"
	path := filepath.Join("testdata", "golden", name+".txt")
	if *update {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			h.t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(got), 0o644); err != nil {
			h.t.Fatal(err)
		}
		return
	}
	want, err := os.ReadFile(path)
	if err != nil {
		h.t.Fatalf("%v (run go test -update)", err)
	}
	if got != string(want) {
		h.t.Errorf("screen %s differs from %s (go test -update to accept):\n%s", name, path, got)
	}
}

// quickToSummary goes from the language screen to the summary with the
// defaults and a password.
func (h *harness) quickToSummary() {
	h.t.Helper()
	h.keys("enter", "enter", "enter", "enter", "down", "down", "=password1", "down", "=password1", "enter", "enter", "enter")
	h.want(scrSummary)
}

// walkToSummary fills every screen with valid answers (data partition and a
// static address included) and stops on the summary.
func (h *harness) walkToSummary(goldenPrefix string) {
	h.t.Helper()
	g := func(name string) {
		if goldenPrefix != "" {
			h.golden(goldenPrefix + name)
		}
	}
	g("01-language")
	h.keys("enter")
	h.want(scrDisk)
	g("02-disk")
	h.keys("enter")
	h.want(scrLayout)
	h.keys("space", "down")
	h.typeField("100")
	g("03-layout")
	h.keys("enter", "enter")
	h.want(scrHost)
	h.typeField("media-box")
	g("04-host")
	h.keys("enter")
	h.want(scrUser)
	h.keys("down", "=Denys", "down", "=correct horse", "down", "=correct horse")
	g("05-user")
	h.keys("enter")
	h.want(scrNet)
	h.keys("right", "down", "right", "down", "=192.168.1.20/24", "down", "=192.168.1.1", "down", "=192.168.1.1, 1.1.1.1")
	g("06-network")
	h.keys("enter")
	h.want(scrSummary)
	g("07-summary")
}

func TestGoldenScreensUkrainian(t *testing.T) {
	h := newHarness(t, "01-live-usb-nas", Options{})
	h.walkToSummary("uk-")
	h.keys("=sda", "enter")
	h.want(scrSummary)
	h.golden("uk-08-summary-wrong-name")
	h.keys("ctrl+u", "=nvme0n1", "enter")
	h.want(scrDone)
	h.golden("uk-09-done")
	h.keys("down", "enter")
	h.want(scrSave)
	h.golden("uk-10-save")
	h.keys("ctrl+c")
	h.golden("uk-11-quit")
}

func TestGoldenScreensEnglishAndRussian(t *testing.T) {
	for i, lang := range []string{"en", "ru"} {
		t.Run(lang, func(t *testing.T) {
			h := newHarness(t, "01-live-usb-nas", Options{})
			for range i + 1 {
				h.keys("down")
			}
			h.keys("enter")
			h.golden(lang + "-02-disk")
			h.keys("enter", "enter", "enter", "enter", "enter", "=correct horse", "down", "=correct horse", "enter", "enter", "enter")
			h.want(scrSummary)
			h.golden(lang + "-07-summary")
		})
	}
}

func TestGoldenDiskWithData(t *testing.T) {
	h := newHarness(t, "02-mdadm-reinstall", Options{Lang: "en"})
	h.keys("enter")
	h.golden("en-02-disk-with-data")
}

func TestTypedConfirmation(t *testing.T) {
	h := newHarness(t, "01-live-usb-nas", Options{})
	h.walkToSummary("")
	for _, wrong := range []string{"", "sda", "nvme0n", "NVME0N1", "yes"} {
		h.keys("ctrl+u")
		if wrong != "" {
			h.keys("=" + wrong)
		}
		h.keys("enter")
		h.want(scrSummary)
		if len(h.env.installed) != 0 {
			t.Fatalf("installed after typing %q", wrong)
		}
		if !strings.Contains(h.m.msg, "nvme0n1") {
			t.Errorf("no hint after %q: %q", wrong, h.m.msg)
		}
	}
	h.keys("ctrl+u", "=/dev/nvme0n1", "enter")
	h.want(scrDone)
	if len(h.env.installed) != 1 {
		t.Fatalf("installs: %d", len(h.env.installed))
	}
	a := h.env.installed[0]
	want := answers.Answers{
		Version:  1,
		Disk:     answers.Disk{Path: "/dev/nvme0n1", DataPartition: true, RootSizeGiB: 100, DataPool: "media"},
		Hostname: "media-box",
		User:     answers.User{Name: "admin", FullName: "Denys", Password: "correct horse"},
		Network: answers.Network{Mode: "static", Interface: "enp1s0", Address: "192.168.1.20/24",
			Gateway: "192.168.1.1", DNS: []string{"192.168.1.1", "1.1.1.1"}},
		Image: answers.Image{Channel: "stable", Target: "ghcr.io/denishumen/alpha-os:stable",
			Source: "containers-storage:ghcr.io/denishumen/alpha-os:stable", Runner: "podman"},
		Language: "uk",
	}
	if got, exp := fmt.Sprintf("%+v", *a), fmt.Sprintf("%+v", want); got != exp {
		t.Errorf("installed answers\n got %s\nwant %s", got, exp)
	}
	if err := a.Validate(); err != nil {
		t.Error(err)
	}
}

func TestBootDiskRefused(t *testing.T) {
	h := newHarness(t, "01-live-usb-nas", Options{Lang: "en"})
	h.keys("enter")
	for _, d := range h.m.cands {
		if d.Path == "/dev/sdd" || strings.HasPrefix(d.Path, "/dev/loop") || strings.HasPrefix(d.Path, "/dev/zram") {
			t.Errorf("%s offered", d.Path)
		}
	}
	if len(h.m.cands) != 4 {
		t.Errorf("candidates: %d, want 4", len(h.m.cands))
	}
	err := h.m.chooseDisk("/dev/sdd")
	if err == nil || !strings.Contains(err.Error(), "boots from this disk") {
		t.Fatalf("boot disk: %v", err)
	}
	if err := h.m.chooseDisk("/dev/nvme0n1p1"); err == nil {
		t.Error("a partition was accepted")
	}
	if !strings.Contains(h.m.View(), "/dev/sdd: the installer boots from it") {
		t.Errorf("boot media not explained:\n%s", h.m.View())
	}

	// The running system's own disk (installer started on a workstation).
	h = newHarness(t, "10-workstation-host", Options{Lang: "en"})
	h.keys("enter")
	for _, b := range h.m.blocked {
		if b.disk.ExcludeReason == disks.ExcludeBootMedia {
			if err := h.m.chooseDisk(b.disk.Path); err == nil {
				t.Errorf("system disk %s accepted", b.disk.Path)
			}
			return
		}
	}
	t.Error("no boot disk in the workstation snapshot")
}

func TestMountedDiskNotOffered(t *testing.T) {
	h := newHarness(t, "04-qemu-ci", Options{Lang: "en"})
	h.env.mutate = func(list []disks.Disk) []disks.Disk {
		for i := range list {
			if list[i].Path == "/dev/sda" {
				list[i].Findings = append(list[i].Findings,
					disks.Finding{Kind: disks.FindingMounted, Device: "/dev/sda1", Detail: "/mnt/old"})
			}
		}
		return list
	}
	h.keys("enter")
	for _, d := range h.m.cands {
		if d.Path == "/dev/sda" {
			t.Fatal("mounted /dev/sda offered")
		}
	}
	if !strings.Contains(h.m.View(), "/dev/sda1 is mounted at /mnt/old") {
		t.Errorf("mounted disk not explained:\n%s", h.m.View())
	}
	if err := h.m.chooseDisk("/dev/sda"); err == nil || !strings.Contains(err.Error(), "mounted") {
		t.Errorf("mounted disk: %v", err)
	}
}

func TestNoDisks(t *testing.T) {
	h := newHarness(t, "03-vm-virtio", Options{Lang: "en"})
	h.env.mutate = func(list []disks.Disk) []disks.Disk {
		for i := range list {
			list[i].Excluded, list[i].ExcludeReason = true, disks.ExcludeTooSmall
		}
		return list
	}
	h.keys("enter", "enter")
	h.want(scrDisk)
	if !strings.Contains(h.m.View(), "There is no disk") {
		t.Errorf("no explanation:\n%s", h.m.View())
	}
}

func TestNavigationBack(t *testing.T) {
	h := newHarness(t, "04-qemu-ci", Options{Lang: "en"})
	h.keys("enter", "enter")
	h.want(scrLayout)
	h.keys("enter", "enter")
	h.want(scrUser)
	h.keys("esc")
	h.want(scrHost)
	h.keys("esc", "esc")
	h.want(scrDisk)
	h.keys("down", "enter")
	h.want(scrLayout)
	if h.m.target.Path != "/dev/sda" {
		t.Errorf("target %s after choosing the second disk", h.m.target.Path)
	}
	h.keys("esc", "esc")
	h.want(scrLang)
	// Language changes keep what was typed.
	h.keys("down", "enter", "enter", "enter")
	h.want(scrHost)
	h.typeField("box")
	h.keys("esc", "esc", "esc", "up", "up", "enter", "enter", "enter")
	h.want(scrHost)
	if v := h.m.val(scrHost, "hostname"); v != "box" {
		t.Errorf("hostname lost: %q", v)
	}
	if h.m.Lang() != "uk" {
		t.Errorf("lang %s", h.m.Lang())
	}
}

func TestValidation(t *testing.T) {
	h := newHarness(t, "04-qemu-ci", Options{Lang: "en"})
	h.keys("enter", "enter")
	// The QEMU NVMe has 21 GB: a 64 GiB system plus data does not fit.
	h.keys("space", "enter", "enter", "enter")
	h.want(scrLayout)
	if !strings.Contains(h.m.msg, "Does not fit") {
		t.Errorf("layout: %q", h.m.msg)
	}
	h.keys("up")
	h.typeField("12")
	h.keys("enter", "enter")
	if !strings.Contains(h.m.msg, "at least 16") {
		t.Errorf("root size: %q", h.m.msg)
	}
	h.keys("up", "up", "space", "enter")
	h.want(scrHost)

	for _, bad := range []string{"Media_Box", "-box", strings.Repeat("a", 64)} {
		h.typeField(bad)
		h.keys("enter")
		h.want(scrHost)
		if !strings.Contains(h.m.msg, "Host name") {
			t.Errorf("hostname %q: %q", bad, h.m.msg)
		}
	}
	h.typeField("nas")
	h.keys("enter")
	h.want(scrUser)

	for _, bad := range []string{"root", "Admin", "1user", "alpha-admin"} {
		h.typeField(bad)
		h.keys("down", "down", "=password1", "down", "=password1", "enter")
		h.want(scrUser)
		if !strings.Contains(h.m.msg, "Name:") {
			t.Errorf("user %q: %q", bad, h.m.msg)
		}
		h.m.forms[scrUser].focus = 0
		h.m.focusForm()
		for _, id := range []string{"pw", "pw2"} {
			h.m.forms[scrUser].get(id).in.SetValue("")
		}
	}
}

func TestPasswords(t *testing.T) {
	h := newHarness(t, "04-qemu-ci", Options{Lang: "en"})
	h.keys("enter", "enter", "enter", "enter")
	h.want(scrUser)
	h.keys("down", "down")
	if !h.m.forms[scrUser].get("pw").in.Focused() {
		t.Fatal("password field not focused")
	}
	h.keys("=secret12", "down", "=secret13", "enter")
	h.want(scrUser)
	if h.m.msg != "The passwords do not match." {
		t.Errorf("mismatch: %q", h.m.msg)
	}
	if v := h.m.View(); strings.Contains(v, "secret1") || !strings.Contains(v, "********") {
		t.Errorf("password shown in clear text:\n%s", v)
	}
	h.keys("up")
	h.typeField("short")
	h.keys("down")
	h.typeField("short")
	h.keys("enter")
	if !strings.Contains(h.m.msg, "at least 8") {
		t.Errorf("short: %q", h.m.msg)
	}
	h.keys("up")
	h.typeField("пароль-кирилицею")
	h.keys("down")
	h.typeField("пароль-кирилицею")
	h.keys("enter")
	h.want(scrNet)
}

func TestStaticNetwork(t *testing.T) {
	h := newHarness(t, "04-qemu-ci", Options{Lang: "en"})
	h.keys("enter", "enter", "enter", "enter", "down", "down", "=password1", "down", "=password1", "enter")
	h.want(scrNet)
	h.keys("right", "down", "down", "=192.168.1.20", "down", "=10.0.0.1", "down", "=8.8.8.8", "enter")
	h.want(scrNet)
	if !strings.HasPrefix(h.m.msg, "Address:") {
		t.Errorf("address: %q", h.m.msg)
	}
	h.keys("up", "up")
	h.typeField("192.168.1.20/24")
	h.keys("down", "down", "enter")
	if !strings.HasPrefix(h.m.msg, "Gateway:") {
		t.Errorf("gateway: %q", h.m.msg)
	}
	h.keys("up")
	h.typeField("192.168.1.1")
	h.keys("down")
	h.typeField("")
	h.keys("enter")
	if !strings.HasPrefix(h.m.msg, "DNS:") {
		t.Errorf("dns: %q", h.m.msg)
	}
	// Back to DHCP: the static fields are hidden and not part of the answers.
	h.m.forms[scrNet].focus = 0
	h.keys("left", "down", "enter")
	h.want(scrSummary)
	if n := h.m.final.Network; n.Mode != "dhcp" || n.Address != "" || n.Interface != "" {
		t.Errorf("network %+v", n)
	}
}

func TestInstallFailureAndRetry(t *testing.T) {
	h := newHarness(t, "04-qemu-ci", Options{Lang: "en", LogPath: "/run/alpha-installer/install.log"})
	h.env.installFn = func(_ *answers.Answers, log io.Writer) (string, error) {
		fmt.Fprintln(log, "==> Partitioning /dev/nvme0n1")
		fmt.Fprintln(log, "sfdisk: cannot open /dev/nvme0n1: Device or resource busy")
		return "", errors.New("partitioning /dev/nvme0n1: sfdisk: exit status 1")
	}
	h.quickToSummary()
	h.want(scrSummary)
	h.keys("=nvme0n1", "enter")
	h.want(scrDone)
	h.golden("en-09-failed")
	if h.m.err == nil {
		t.Fatal("no error")
	}
	h.keys("enter")
	h.want(scrSummary)
	if h.m.confirm.Value() != "" {
		t.Error("confirmation kept after a failure")
	}
	h.env.installFn = nil
	h.keys("=nvme0n1", "enter")
	h.want(scrDone)
	if h.m.err != nil || len(h.env.installed) != 2 {
		t.Errorf("retry: %v, %d installs", h.m.err, len(h.env.installed))
	}
}

func TestProgressScreen(t *testing.T) {
	h := newHarness(t, "04-qemu-ci", Options{Lang: "en", LogPath: "/run/alpha-installer/install.log"})
	release := make(chan struct{})
	h.env.installFn = func(_ *answers.Answers, log io.Writer) (string, error) {
		fmt.Fprintln(log, "==> Partitioning /dev/nvme0n1 (QEMU NVMe Ctrl, 21 GB)")
		fmt.Fprintln(log, "\x1b[1mbold\x1b[0m line")
		fmt.Fprintln(log, "==> Installing containers-storage:ghcr.io/denishumen/alpha-os:stable")
		<-release
		return "", nil
	}
	h.quickToSummary()
	h.keys("=nvme0n1")
	// Drive by hand: the install blocks until released.
	_, wait := h.m.Update(tea.KeyMsg{Type: tea.KeyEnter})
	h.want(scrProgress)
	for range 3 {
		_, wait = h.m.Update(wait())
	}
	h.golden("en-08-progress")
	_, c := h.m.Update(tea.KeyMsg{Type: tea.KeyCtrlC})
	if c != nil || h.m.quitOpen || !strings.Contains(h.m.msg, "running") {
		t.Errorf("ctrl+c during the installation: quit=%v msg=%q", h.m.quitOpen, h.m.msg)
	}
	close(release)
	h.run(wait)
	h.want(scrDone)
	if h.m.logLines[1] != "bold line" {
		t.Errorf("ANSI not stripped: %q", h.m.logLines[1])
	}
}

func TestSavedAnswersUseHash(t *testing.T) {
	h := newHarness(t, "01-live-usb-nas", Options{})
	h.walkToSummary("")
	h.keys("=nvme0n1", "enter")
	h.want(scrDone)
	h.keys("down", "enter")
	h.want(scrSave)
	// USB stick and external disk of the 07 snapshot, then the live system.
	if len(h.m.saves) != 3 || h.m.saves[0].Device != "/dev/sdb1" || h.m.saves[1].Device != "/dev/sdd1" ||
		h.m.saves[2].Path == "" {
		t.Fatalf("save targets %+v", h.m.saves)
	}
	h.keys("down", "enter")
	if len(h.env.saved) != 1 || !strings.Contains(h.m.msg, "/dev/sdd1:/kinolune-answers.json") {
		t.Fatalf("saved %d, msg %q", len(h.env.saved), h.m.msg)
	}
	data := h.env.saved[0]
	if bytes.Contains(data, []byte("correct horse")) || !bytes.Contains(data, []byte(`"password_hash": "$y$`)) {
		t.Errorf("saved file:\n%s", data)
	}
	if bytes.Contains(data, []byte("containers-storage")) {
		t.Errorf("defaults saved:\n%s", data)
	}
	a, err := answers.Load(bytes.NewReader(data))
	if err != nil {
		t.Fatalf("saved file does not load: %v\n%s", err, data)
	}
	if a.Disk.RootSizeGiB != 100 || a.Network.Mode != "static" || a.Language != "uk" {
		t.Errorf("loaded %+v", a)
	}
	h.keys("esc")
	h.want(scrDone)
	h.keys("up", "enter")
	if !h.quit || h.m.Action() != ActionReboot {
		t.Errorf("reboot: quit=%v action=%q", h.quit, h.m.Action())
	}
}

func TestDryRunSavesClearTextWithWarning(t *testing.T) {
	h := newHarness(t, "04-qemu-ci", Options{Lang: "en", DryRun: true})
	h.env.installFn = func(_ *answers.Answers, log io.Writer) (string, error) {
		fmt.Fprintln(log, "Kinolune OS installation plan")
		fmt.Fprintln(log, "Disk:     QEMU NVMe Ctrl, 21 GB")
		fmt.Fprintln(log, "Dry run: nothing was written.")
		return "", nil
	}
	h.quickToSummary()
	h.want(scrSummary)
	if !strings.Contains(h.m.View(), "Dry run: nothing is written") {
		t.Errorf("no dry-run note:\n%s", h.m.View())
	}
	h.keys("=nvme0n1", "enter")
	h.want(scrDone)
	h.golden("en-09-dry-run")
	h.keys("enter")
	h.want(scrSave)
	if !strings.Contains(h.m.View(), "clear text") {
		t.Errorf("no clear-text warning:\n%s", h.m.View())
	}
}

func TestPrefillFromAnswerFile(t *testing.T) {
	f, err := os.Open("../../examples/answers.json")
	if err != nil {
		t.Fatal(err)
	}
	base, err := answers.Load(f)
	f.Close()
	if err != nil {
		t.Fatal(err)
	}
	base.Image = answers.Image{Channel: "beta"}
	base.KernelArgs = []string{"console=ttyS0,115200"}
	h := newHarness(t, "01-live-usb-nas", Options{Base: base})
	if h.m.Lang() != "uk" || h.m.langIdx != 0 {
		t.Errorf("language from the file: %s", h.m.Lang())
	}
	h.keys("enter", "enter")
	h.want(scrLayout)
	if !h.m.forms[scrLayout].get("data").on || h.m.val(scrLayout, "root") != "64" {
		t.Error("layout not pre-filled")
	}
	h.keys("enter", "enter", "enter", "enter")
	h.want(scrUser)
	if !strings.Contains(h.m.View(), "Порожній пароль") {
		t.Errorf("no keep-password hint:\n%s", h.m.View())
	}
	h.keys("enter", "enter", "enter", "enter")
	h.want(scrNet)
	h.keys("enter", "enter", "enter", "enter", "enter")
	h.want(scrSummary)
	a := h.m.final
	if a.User.PasswordHash != base.User.PasswordHash || a.User.Password != "" || a.Hostname != "kinolune" ||
		a.Network.Address != "192.168.1.20/24" || a.Image.Channel != "beta" || len(a.KernelArgs) != 1 {
		t.Errorf("prefilled answers %+v", a)
	}
}

func TestQuitDialog(t *testing.T) {
	h := newHarness(t, "04-qemu-ci", Options{Lang: "en"})
	h.keys("ctrl+c")
	if !h.m.quitOpen {
		t.Fatal("no dialog")
	}
	h.keys("enter")
	if h.quit || h.m.quitOpen {
		t.Fatal("continue quit")
	}
	h.keys("ctrl+c", "down", "enter")
	if !h.quit || h.m.Action() != ActionMenu {
		t.Errorf("menu: quit=%v action=%q", h.quit, h.m.Action())
	}
}

func TestSerialConsoleSize(t *testing.T) {
	h := newHarness(t, "04-qemu-ci", Options{})
	h.send(tea.WindowSizeMsg{Width: 0, Height: 0})
	if h.m.w != 80 || h.m.h != 24 {
		t.Errorf("size %dx%d", h.m.w, h.m.h)
	}
	h.send(tea.WindowSizeMsg{Width: 128, Height: 48})
	if n := len(strings.Split(h.m.View(), "\n")); n != 48 {
		t.Errorf("%d lines on 128x48", n)
	}
}

func TestTextsComplete(t *testing.T) {
	for k, v := range texts {
		for i, s := range v {
			if s == "" {
				t.Errorf("%s: no %s text", k, Languages[i].Code)
			}
			if strings.ContainsAny(s, "…←↑→↓—") {
				t.Errorf("%s/%s: glyph the console font may lack: %q", k, Languages[i].Code, s)
			}
		}
	}
	for _, k := range []disks.ExcludeReason{disks.ExcludeVirtual, disks.ExcludeOptical, disks.ExcludeBootMedia,
		disks.ExcludeNoMedia, disks.ExcludeReadOnly, disks.ExcludeTooSmall, disks.ExcludeHWArea, disks.ExcludeNotDisk} {
		if _, ok := texts["excl."+string(k)]; !ok {
			t.Errorf("no text for %s", k)
		}
	}
	for _, k := range []disks.FindingKind{disks.FindingPartitionTable, disks.FindingFilesystem, disks.FindingRAIDMember,
		disks.FindingLVMMember, disks.FindingLUKS, disks.FindingZFSMember, disks.FindingSwap, disks.FindingWindows,
		disks.FindingEFISystem, disks.FindingMounted, disks.FindingSmartFailed} {
		if _, ok := texts["find."+string(k)]; !ok {
			t.Errorf("no text for %s", k)
		}
	}
}

func TestReadInterfaces(t *testing.T) {
	root := t.TempDir()
	mk := func(name string, files map[string]string, dirs ...string) {
		d := filepath.Join(root, "sys", "class", "net", name)
		for _, x := range dirs {
			if err := os.MkdirAll(filepath.Join(d, x), 0o755); err != nil {
				t.Fatal(err)
			}
		}
		for f, v := range files {
			if err := os.WriteFile(filepath.Join(d, f), []byte(v+"\n"), 0o644); err != nil {
				t.Fatal(err)
			}
		}
	}
	mk("lo", map[string]string{"type": "772"}, ".")
	mk("enp2s0", map[string]string{"type": "1", "address": "52:54:00:00:00:02", "carrier": "0"}, "device")
	mk("enp1s0", map[string]string{"type": "1", "address": "52:54:00:00:00:01", "carrier": "1"}, "device")
	mk("wlp3s0", map[string]string{"type": "1"}, "device", "wireless")
	mk("virbr0", map[string]string{"type": "1"}, ".")
	got := ReadInterfaces(root)
	want := []Iface{{"enp1s0", "52:54:00:00:00:01", true}, {"enp2s0", "52:54:00:00:00:02", false}}
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Errorf("got %v, want %v", got, want)
	}
}

// recRunner records commands without running them.
type recRunner struct{ cmds []string }

func (r *recRunner) Run(_ context.Context, c install.Cmd) ([]byte, error) {
	r.cmds = append(r.cmds, c.String())
	return nil, nil
}
func (*recRunner) LookPath(string) bool { return true }

func TestSaveToPartition(t *testing.T) {
	list, err := disks.Probe(disks.Dir(fixtures+"07-usb-mix"), disks.Options{})
	if err != nil {
		t.Fatal(err)
	}
	// The boot stick, the empty card reader and the install target are never offered.
	targets := FindSaveTargets(list, "/dev/sdb", "")
	if len(targets) != 1 || targets[0].Device != "/dev/sdd1" || targets[0].FSType != "vfat" || targets[0].Label != "MUSIC" {
		t.Fatalf("targets %+v", targets)
	}
	r := &recRunner{}
	mnt := t.TempDir()
	where, err := SaveToPartition(context.Background(), r, targets[0], mnt, []byte("{}\n"))
	if err != nil {
		t.Fatal(err)
	}
	if where != "/dev/sdd1:/kinolune-answers.json" {
		t.Errorf("where %q", where)
	}
	if got := strings.Join(r.cmds, "; "); got != "mount -t vfat /dev/sdd1 "+mnt+"; sync; umount "+mnt {
		t.Errorf("commands: %s", got)
	}
	if b, err := os.ReadFile(filepath.Join(mnt, AnswerFileName)); err != nil || string(b) != "{}\n" {
		t.Errorf("file: %q %v", b, err)
	}
	local := filepath.Join(t.TempDir(), "root", AnswerFileName)
	if _, err := SaveToPartition(context.Background(), r, SaveTarget{Path: local}, "", []byte("{}\n")); err != nil {
		t.Fatal(err)
	}
	if st, err := os.Stat(local); err != nil || st.Mode().Perm() != 0o600 {
		t.Errorf("local file: %v %v", st, err)
	}
}
