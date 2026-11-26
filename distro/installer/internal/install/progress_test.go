// SPDX-License-Identifier: MPL-2.0

package install

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"
)

// manualTicker fires when the test says so.
type manualTicker struct{ c chan time.Time }

func (m *manualTicker) C() <-chan time.Time { return m.c }
func (m *manualTicker) Stop()               {}

// bootcRunner wraps the fake runner: bootc prints its phases to Output
// while the "disk" fills up, one tick of the meter per phase.
type bootcRunner struct {
	*fakeRunner
	written *int64
	mu      *sync.Mutex
	tick    func()
	fail    bool
}

func (b bootcRunner) Run(ctx context.Context, c Cmd) ([]byte, error) {
	if c.Name != "podman" || c.Output == nil {
		return b.fakeRunner.Run(ctx, c)
	}
	for _, n := range []int64{100e6, 300e6, 550e6} {
		b.mu.Lock()
		*b.written += n
		b.mu.Unlock()
		b.tick()
	}
	if b.fail {
		fmt.Fprintln(c.Output, "error: writing objects: No space left on device")
		return nil, errors.New("podman: exit status 1")
	}
	fmt.Fprint(c.Output, "Deploying container image...done\r\nInstalling bootloader via bootupd\n")
	return b.fakeRunner.Run(ctx, c)
}

type recorder struct {
	mu  sync.Mutex
	evs []Event
}

func (r *recorder) add(e Event) { r.mu.Lock(); r.evs = append(r.evs, e); r.mu.Unlock() }

func (r *recorder) kinds() string {
	r.mu.Lock()
	defer r.mu.Unlock()
	var out []string
	for _, e := range r.evs {
		if e.Kind != EventProgress {
			out = append(out, e.Kind+":"+string(e.Step))
		}
	}
	return strings.Join(out, " ")
}

func progressSetup(t *testing.T, fail bool) (setup, *recorder, *bytes.Buffer) {
	s := newSetup(t, "04-qemu-ci", qemuAnswers)
	rec := &recorder{}
	var jsonl bytes.Buffer
	s.in.Progress = Tee(rec.add, JSONLines(&jsonl))
	var written int64
	var mu sync.Mutex
	tk := &manualTicker{c: make(chan time.Time)}
	s.in.Ticker = func(time.Duration) Ticker { return tk }
	s.in.DiskWritten = func() (int64, error) { mu.Lock(); defer mu.Unlock(); return written, nil }
	s.in.PayloadSize = func() int64 { return 1e9 } // 1.05 GB expected on the disk
	clock := time.Date(2026, 11, 26, 10, 0, 0, 0, time.UTC)
	var cmu sync.Mutex
	s.in.Now = func() time.Time { cmu.Lock(); defer cmu.Unlock(); clock = clock.Add(time.Second); return clock }
	s.in.Runner = bootcRunner{fakeRunner: s.fake, written: &written, mu: &mu, fail: fail,
		// The second tick is taken only after the first one was sampled.
		tick: func() { tk.c <- time.Time{}; tk.c <- time.Time{} }}
	return s, rec, &jsonl
}

func TestRunReportsEveryTask(t *testing.T) {
	s, rec, jsonl := progressSetup(t, false)
	if err := s.in.Run(context.Background()); err != nil {
		t.Fatalf("%v\n%s", err, s.log)
	}
	want := "start:partition done:partition start:format done:format start:copy done:copy " +
		"start:bootloader done:bootloader start:configure done:configure start:finalize done:finalize"
	if got := rec.kinds(); got != want {
		t.Errorf("events\n got %s\nwant %s", got, want)
	}
	// The copy counts bytes written since it started, against the payload.
	var copied []Event
	for _, e := range rec.evs {
		if e.Step == StepCopy && e.Kind == EventProgress {
			copied = append(copied, e)
		}
	}
	if len(copied) < 3 || !copied[0].Bytes || copied[0].Done != 100e6 || copied[0].Total != 1.05e9 {
		t.Fatalf("copy progress: %+v", copied)
	}
	if last := copied[len(copied)-1]; last.Done != 950e6 || last.Total != last.Done {
		t.Errorf("the copy does not end at 100 %%: %+v", last)
	}
	// Units of the other tasks reach their totals.
	tr := NewTracker()
	evs, err := ReadEvents(jsonl)
	if err != nil || len(evs) != len(rec.evs) {
		t.Fatalf("recording: %d events, %v", len(evs), err)
	}
	for _, e := range evs {
		tr.Apply(e)
	}
	for _, k := range tr.Tasks {
		if k.State != Finished || k.Fraction() != 1 {
			t.Errorf("%s: state %d, %v", k.ID, k.State, k.Fraction())
		}
	}
	if c := tr.Task(StepConfigure); c.Total != 5 {
		t.Errorf("configure units %d", c.Total)
	}
	if tr.Overall(tr.Last) != 1 || tr.Remaining(tr.Last) != 0 {
		t.Errorf("finished install: overall %v, remaining %v", tr.Overall(tr.Last), tr.Remaining(tr.Last))
	}
}

func TestRunFailedTask(t *testing.T) {
	s, rec, _ := progressSetup(t, true)
	if err := s.in.Run(context.Background()); err == nil {
		t.Fatal("no error")
	}
	got := rec.kinds()
	if !strings.HasSuffix(got, "start:copy fail:copy") || strings.Contains(got, "finalize") {
		t.Errorf("events: %s", got)
	}
	last := rec.evs[len(rec.evs)-1]
	if !strings.Contains(last.Error, "bootc install") {
		t.Errorf("fail event: %+v", last)
	}
}

func TestCopyBeyondEstimateStaysBelow100(t *testing.T) {
	in := &Installer{Now: time.Now}
	in.started = in.Now()
	var evs []Event
	in.Progress = func(e Event) { evs = append(evs, e) }
	n := int64(0)
	m := &copyMeter{in: in, total: 1000, read: func() (int64, error) { return n, nil }}
	for _, v := range []int64{500, 990, 1500} {
		n = v
		m.sample()
	}
	last := evs[len(evs)-1]
	if last.Done != 1500 || last.Total <= last.Done || last.Done*100/last.Total < 97 {
		t.Errorf("beyond the estimate: %+v", last)
	}
	n = 1200 // counters never go back
	m.sample()
	if e := evs[len(evs)-1]; e.Done != 1500 {
		t.Errorf("went back: %+v", e)
	}
}

func TestTrackerSpeedAndETA(t *testing.T) {
	tr := NewTracker()
	tr.Apply(Event{At: 0, Kind: EventStart, Step: StepPartition, Total: 6})
	tr.Apply(Event{At: 2000, Kind: EventDone, Step: StepPartition})
	tr.Apply(Event{At: 2000, Kind: EventStart, Step: StepFormat, Total: 4})
	tr.Apply(Event{At: 7000, Kind: EventDone, Step: StepFormat})
	tr.Apply(Event{At: 7000, Kind: EventStart, Step: StepCopy, Total: 1000e6, Bytes: true})
	for i := int64(1); i <= 20; i++ {
		// 10 MB/s for 10 s, then 50 MB/s: the speed follows the last 5 s.
		v := min(i, 10)*10e6 + max(i-10, 0)*50e6
		tr.Apply(Event{At: 7000 + i*1000, Kind: EventProgress, Step: StepCopy, Done: v, Total: 1000e6, Bytes: true})
	}
	c := tr.Task(StepCopy)
	if c.Speed != 50e6 || c.Done != 600e6 {
		t.Fatalf("speed %v done %v", c.Speed, c.Done)
	}
	eta, ok := c.ETA()
	if !ok || eta != 8*time.Second {
		t.Errorf("copy ETA %v %v", eta, ok)
	}
	// 8 s of copy + bootloader, configure and finalize expected.
	want := 8*time.Second + Expected[StepBootloader] + Expected[StepConfigure] + Expected[StepFinalize]
	if r := tr.Remaining(27000); r != want {
		t.Errorf("remaining %v, want %v", r, want)
	}
	if r := tr.Remaining(30000); r != want-3*time.Second {
		t.Errorf("remaining 3 s later %v", r)
	}
	if f := tr.Overall(27000); f < 0.6 || f > 0.7 { // 27 s of 27 + 15
		t.Errorf("overall %v", f)
	}
	tr.Apply(Event{At: 28000, Kind: EventFail, Step: StepCopy, Error: "boom"})
	if k := tr.Failed(); k == nil || k.ID != StepCopy || k.Elapsed(99000) != 21*time.Second {
		t.Errorf("failed task %+v", k)
	}
}

func TestTrackerUnknownAmount(t *testing.T) {
	tr := NewTracker()
	tr.Apply(Event{At: 0, Kind: EventStart, Step: StepBootloader})
	k := tr.Task(StepBootloader)
	if k.Fraction() != -1 {
		t.Errorf("fraction %v", k.Fraction())
	}
	if _, ok := k.ETA(); ok {
		t.Error("ETA without an amount")
	}
	// Past its expected time a task still claims at least a second.
	if r := tr.Remaining(60000); r < time.Second {
		t.Errorf("remaining %v", r)
	}
}
