// SPDX-License-Identifier: MPL-2.0

package tui

import (
	"bytes"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/x/exp/teatest"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
	"github.com/DenisHumen/kinolune/distro/installer/internal/install"
)

// recording is the progress stream of the TUI install of the ISO in QEMU
// (DIST-034): /run/alpha-installer/progress.jsonl of the live system.
const recording = "qemu-install.jsonl"

func loadRecording(t *testing.T) []install.Event {
	t.Helper()
	f, err := os.Open(filepath.Join("testdata", "progress", recording))
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	evs, err := install.ReadEvents(f)
	if err != nil || len(evs) == 0 {
		t.Fatalf("recording: %d events, %v", len(evs), err)
	}
	return evs
}

// index of the first event that satisfies ok.
func index(t *testing.T, evs []install.Event, ok func(install.Event) bool) int {
	t.Helper()
	for i, e := range evs {
		if ok(e) {
			return i
		}
	}
	t.Fatal("no such event in the recording")
	return -1
}

// replayHarness drives the progress screen with recorded events: the screen
// clock is the time of the latest event, the installation stops at every
// checkpoint until the test has looked at the screen.
type replayHarness struct {
	*harness
	wait  tea.Cmd
	gate  chan struct{}
	stops map[int]bool
	sent  int
}

func newReplay(t *testing.T, lang string, evs []install.Event, stops []int, err error) *replayHarness {
	var m *Model
	opt := Options{Lang: lang, LogPath: "/run/alpha-installer/install.log", Now: func() time.Time {
		if m != nil && m.tasks != nil {
			return t0.Add(time.Duration(m.tasks.Last) * time.Millisecond)
		}
		return t0
	}}
	h := newHarness(t, "04-qemu-ci", opt)
	m = h.m
	r := &replayHarness{harness: h, gate: make(chan struct{}), stops: map[int]bool{}}
	for _, s := range stops {
		r.stops[s] = true
	}
	h.env.installFn = func(_ *answers.Answers, log io.Writer) (string, error) {
		sink, ok := log.(ProgressSink)
		if !ok {
			t.Error("the log writer carries no progress")
			return "", nil
		}
		for i, e := range evs {
			sink.Progress(e)
			if r.stops[i] {
				<-r.gate
			}
		}
		return "", err
	}
	h.quickToSummary()
	h.keys("=nvme0n1")
	_, r.wait = h.m.Update(tea.KeyMsg{Type: tea.KeyEnter})
	h.want(scrProgress)
	return r
}

// upTo shows events up to and including index i.
func (r *replayHarness) upTo(i int) {
	for r.sent <= i {
		_, r.wait = r.m.Update(r.wait())
		r.sent++
	}
}

// release lets the installation run past a checkpoint.
func (r *replayHarness) release() { r.gate <- struct{}{} }

func TestProgressReplay(t *testing.T) {
	evs := loadRecording(t)
	midCopy := index(t, evs, func(e install.Event) bool {
		return e.Step == install.StepCopy && e.Kind == install.EventProgress && e.Done*100 >= e.Total*40
	})
	boot := index(t, evs, func(e install.Event) bool {
		return e.Step == install.StepBootloader && e.Kind == install.EventStart
	})
	conf := index(t, evs, func(e install.Event) bool {
		return e.Step == install.StepConfigure && e.Kind == install.EventProgress && e.Done >= 2
	})
	for _, lang := range []string{"en", "uk", "ru"} {
		t.Run(lang, func(t *testing.T) {
			r := newReplay(t, lang, evs, []int{midCopy, boot, conf}, nil)
			r.upTo(midCopy)
			r.golden(lang + "-08-tasks-copy")
			c := r.m.tasks.Task(install.StepCopy)
			if c.Speed <= 0 || c.Fraction() < 0.4 {
				t.Errorf("copy: speed %v, %v", c.Speed, c.Fraction())
			}
			r.release()
			r.upTo(boot)
			if lang == "en" {
				r.golden("en-08-tasks-bootloader")
			}
			r.release()
			r.upTo(conf)
			if lang == "en" {
				r.golden("en-08-tasks-configure")
				// L shows the command log and back.
				r.keys("=l")
				if !r.m.showLog || !strings.Contains(r.m.View(), "Log: /run/alpha-installer/install.log") {
					t.Error("L did not open the log")
				}
				r.keys("=l")
				if r.m.showLog {
					t.Error("L did not close the log")
				}
			}
			r.release()
			r.run(r.wait)
			r.want(scrDone)
			for _, k := range r.m.tasks.Tasks {
				if k.State != install.Finished {
					t.Errorf("%s not finished", k.ID)
				}
			}
			if lang == "en" {
				r.golden("en-09-done-after-progress")
			}
		})
	}
}

func TestProgressFailedStep(t *testing.T) {
	evs := loadRecording(t)
	cut := index(t, evs, func(e install.Event) bool {
		return e.Step == install.StepCopy && e.Kind == install.EventProgress && e.Done*100 >= e.Total*60
	})
	failed := append(append([]install.Event(nil), evs[:cut+1]...), install.Event{
		At: evs[cut].At + 400, Kind: install.EventFail, Step: install.StepCopy,
		Error: "bootc install: podman: exit status 1: error: writing objects: No space left on device",
	})
	r := newReplay(t, "en", failed, []int{len(failed) - 1}, errFailed)
	r.upTo(len(failed) - 1)
	r.golden("en-08-tasks-failed")
	r.release()
	r.run(r.wait)
	r.want(scrDone)
	r.golden("en-09-failed-step")
}

var errFailed = errors.New("bootc install: podman: exit status 1: error: writing objects: No space left on device")

// TestProgressTeatest runs the whole recorded installation through the
// Bubble Tea runtime (teatest) and checks the final screen.
func TestProgressTeatest(t *testing.T) {
	evs := loadRecording(t)
	// The harness walks to the summary; the runtime takes over from there.
	var m *Model
	opt := Options{Lang: "en", Version: "0.1.0", Now: func() time.Time {
		if m != nil && m.tasks != nil {
			return t0.Add(time.Duration(m.tasks.Last) * time.Millisecond)
		}
		return t0
	}}
	h2 := newHarness(t, "04-qemu-ci", opt)
	m = h2.m
	h2.env.installFn = func(_ *answers.Answers, log io.Writer) (string, error) {
		for _, e := range evs {
			log.(ProgressSink).Progress(e)
		}
		return "", nil
	}
	h2.quickToSummary()
	h2.keys("=nvme0n1")
	tm := teatest.NewTestModel(t, m, teatest.WithInitialTermSize(80, 24))
	tm.Send(tea.KeyMsg{Type: tea.KeyEnter})
	teatest.WaitFor(t, tm.Output(), func(b []byte) bool {
		return bytes.Contains(b, []byte("Kinolune OS is installed"))
	}, teatest.WithDuration(10*time.Second))
	if err := tm.Quit(); err != nil {
		t.Fatal(err)
	}
	fm := tm.FinalModel(t, teatest.WithFinalTimeout(5*time.Second)).(*Model)
	if fm.scr != scrDone || fm.err != nil {
		t.Fatalf("screen %d, err %v", fm.scr, fm.err)
	}
	for _, k := range fm.tasks.Tasks {
		if k.State != install.Finished {
			t.Errorf("%s not finished", k.ID)
		}
	}
	if got, want := fm.elapsed(), (time.Duration(evs[len(evs)-1].At) * time.Millisecond).Round(time.Second).String(); got != want {
		t.Errorf("elapsed %s, want %s", got, want)
	}
}
