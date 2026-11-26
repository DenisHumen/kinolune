// SPDX-License-Identifier: MPL-2.0

package install

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

// StepID names one task of the installation as the progress screen shows
// it (DIST-034): every task gets its own line with a bar, like pacman.
type StepID string

// The tasks of an installation, in order.
const (
	StepPartition  StepID = "partition"
	StepFormat     StepID = "format"
	StepCopy       StepID = "copy"
	StepBootloader StepID = "bootloader"
	StepConfigure  StepID = "configure"
	StepFinalize   StepID = "finalize"
)

// Steps lists the tasks in the order they run.
var Steps = []StepID{StepPartition, StepFormat, StepCopy, StepBootloader, StepConfigure, StepFinalize}

// Event kinds.
const (
	EventStart    = "start"
	EventProgress = "progress"
	EventDone     = "done"
	EventFail     = "fail"
)

// Event is one change of a task. A stream of events (the JSON lines in
// ProgressLogPath) is everything a progress screen needs: speed and ETA are
// computed from it by Tracker, so a recorded stream replays the screen.
type Event struct {
	// At is the time since the installation started, in milliseconds.
	At   int64  `json:"t"`
	Kind string `json:"kind"`
	Step StepID `json:"step"`
	// Done of Total units; Total 0 means the amount of work is unknown
	// (an indeterminate bar). Bytes: the units are bytes (speed, ETA).
	Done  int64 `json:"done,omitempty"`
	Total int64 `json:"total,omitempty"`
	Bytes bool  `json:"bytes,omitempty"`
	// Detail is what the task works on right now (a device, a file system).
	Detail string `json:"detail,omitempty"`
	Error  string `json:"error,omitempty"`
}

// ProgressLogPath is where the live system keeps the event stream.
const ProgressLogPath = "/run/alpha-installer/progress.jsonl"

// ReadEvents parses a JSON-lines event stream (a recording).
func ReadEvents(r io.Reader) ([]Event, error) {
	var out []Event
	dec := json.NewDecoder(r)
	for dec.More() {
		var e Event
		if err := dec.Decode(&e); err != nil {
			return out, err
		}
		out = append(out, e)
	}
	return out, nil
}

// JSONLines returns a sink that appends each event to w as one JSON line.
// It is safe for concurrent use.
func JSONLines(w io.Writer) func(Event) {
	var mu sync.Mutex
	return func(e Event) {
		b, err := json.Marshal(e)
		if err != nil {
			return
		}
		mu.Lock()
		defer mu.Unlock()
		_, _ = w.Write(append(b, '\n'))
	}
}

// Tee sends every event to all sinks; nil sinks are skipped.
func Tee(sinks ...func(Event)) func(Event) {
	return func(e Event) {
		for _, s := range sinks {
			if s != nil {
				s(e)
			}
		}
	}
}

// --- emitting --------------------------------------------------------------

func (in *Installer) emit(e Event) {
	in.progMu.Lock()
	defer in.progMu.Unlock()
	switch e.Kind {
	case EventStart:
		in.cur, in.curDone, in.curTotal, in.curNotes = e.Step, 0, e.Total, 0
	case EventDone, EventFail:
		if in.cur == e.Step {
			in.cur = ""
		}
	}
	if in.Progress == nil {
		return
	}
	e.At = in.Now().Sub(in.started).Milliseconds()
	in.Progress(e)
}

// begin starts task s with total units of work (0: unknown).
func (in *Installer) begin(s StepID, total int64) {
	in.emit(Event{Kind: EventStart, Step: s, Total: total})
}

// unitDone counts one finished unit of task s if it is running; detail
// names what was done.
func (in *Installer) unitDone(s StepID, detail string) {
	in.progMu.Lock()
	if in.cur != s {
		in.progMu.Unlock()
		return
	}
	if in.curDone < in.curTotal {
		in.curDone++
	}
	done, total := in.curDone, in.curTotal
	in.progMu.Unlock()
	in.emit(Event{Kind: EventProgress, Step: s, Done: done, Total: total, Detail: detail})
}

// note is a "==>" line of the configure task: each one starts a unit, so the
// ones before it are done.
func (in *Installer) note(text string) {
	in.progMu.Lock()
	s := in.cur
	if s != StepConfigure {
		in.progMu.Unlock()
		return
	}
	done := int64(in.curNotes)
	in.curNotes++
	if done > in.curTotal {
		done = in.curTotal
	}
	in.curDone = done
	total := in.curTotal
	in.progMu.Unlock()
	in.emit(Event{Kind: EventProgress, Step: s, Done: done, Total: total, Detail: text})
}

// finish closes task s at 100 %.
func (in *Installer) finish(s StepID) {
	in.progMu.Lock()
	total := in.curTotal
	cur := in.cur
	in.progMu.Unlock()
	if cur == s && total > 0 {
		in.emit(Event{Kind: EventProgress, Step: s, Done: total, Total: total})
	}
	in.emit(Event{Kind: EventDone, Step: s})
}

// failCurrent marks the running task failed.
func (in *Installer) failCurrent(err error) {
	in.progMu.Lock()
	cur := in.cur
	in.progMu.Unlock()
	if cur != "" {
		in.emit(Event{Kind: EventFail, Step: cur, Error: err.Error()})
	}
}

// configureUnits is the number of "==>" lines configure prints: /var,
// hostname, network (unless DHCP on any port), user, SELinux.
func (in *Installer) configureUnits() int64 {
	if NMConnection(in.Answers.Network, "x") == "" {
		return 4
	}
	return 5
}

// --- the image copy --------------------------------------------------------

// CopyRatio is the bytes the copy writes to the disk per byte of the
// unpacked payload: ostree objects plus the deployment, less what btrfs
// zstd:1 saves. Measured on the QEMU install of the ISO (DIST-034): 2.22 GB
// written for a 2.16 GB payload. The bar never claims 100 % before bootc
// moves on to the bootloader.
const CopyRatio = 1.05

// sampleEvery is how often the copy measures the bytes written to the disk.
const sampleEvery = time.Second

// diskWritten reads the bytes written to a whole disk from
// /sys/block/<name>/stat (field 7: sectors of 512 bytes).
func diskWritten(name string) (int64, error) {
	b, err := os.ReadFile(filepath.Join("/sys/block", name, "stat"))
	if err != nil {
		return 0, err
	}
	f := strings.Fields(string(b))
	if len(f) < 7 {
		return 0, fmt.Errorf("short %s/stat", name)
	}
	n, err := strconv.ParseInt(f[6], 10, 64)
	return n * 512, err
}

// payloadSize is the unpacked size of the image that will be installed,
// or 0 when it is unknown (the bar is then indeterminate, with speed).
func (in *Installer) payloadSize() int64 {
	if in.PayloadSize == nil {
		return 0
	}
	return in.PayloadSize()
}

// copyMeter follows the image copy: it samples the bytes written to the
// disk and watches the bootc output for the bootloader phase.
type copyMeter struct {
	in    *Installer
	total int64
	base  int64
	read  func() (int64, error)

	mu         sync.Mutex
	last       int64
	bootloader bool
	stop       chan struct{}
	stopped    chan struct{}

	wmu sync.Mutex
	buf []byte
}

func (in *Installer) startCopy() *copyMeter {
	total := int64(float64(in.payloadSize()) * CopyRatio)
	in.emit(Event{Kind: EventStart, Step: StepCopy, Total: total, Bytes: true})
	read := in.DiskWritten
	if read == nil {
		name := filepath.Base(in.Disk.Path)
		read = func() (int64, error) { return diskWritten(name) }
	}
	m := &copyMeter{in: in, total: total, read: read, stop: make(chan struct{}), stopped: make(chan struct{})}
	if b, err := read(); err == nil {
		m.base = b
	} else {
		m.read = nil
	}
	go m.loop()
	return m
}

func (m *copyMeter) loop() {
	defer close(m.stopped)
	if m.read == nil {
		return
	}
	t := m.in.Ticker(sampleEvery)
	defer t.Stop()
	for {
		select {
		case <-m.stop:
			return
		case <-t.C():
			m.sample()
		}
	}
}

func (m *copyMeter) sample() {
	if m.read == nil {
		return
	}
	b, err := m.read()
	if err != nil {
		return
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.bootloader {
		return
	}
	done := b - m.base
	if done < m.last {
		done = m.last
	}
	m.last = done
	total := m.total
	// More written than expected (a less compressible image): keep the bar
	// honest below 100 % until bootc reports the deployment.
	if total > 0 && done > total*98/100 {
		total = done * 100 / 98
		m.total = total
	}
	m.in.emit(Event{Kind: EventProgress, Step: StepCopy, Done: done, Total: total, Bytes: true})
}

// Write receives the bootc output (stdout and stderr) line by line.
// stdout and stderr arrive from two goroutines.
func (m *copyMeter) Write(p []byte) (int, error) {
	m.wmu.Lock()
	defer m.wmu.Unlock()
	m.buf = append(m.buf, p...)
	for {
		i := bytes.IndexAny(m.buf, "\r\n")
		if i < 0 {
			break
		}
		m.line(string(m.buf[:i]))
		m.buf = m.buf[i+1:]
	}
	return len(p), nil
}

// bootloaderMarkers are bootc's messages when it leaves the image deployment
// for the bootloader (bootupd).
var bootloaderMarkers = []string{"bootupctl", "Installing bootloader"}

func (m *copyMeter) line(s string) {
	for _, k := range bootloaderMarkers {
		if strings.Contains(s, k) {
			m.toBootloader()
			return
		}
	}
}

// toBootloader closes the copy at 100 % and starts the bootloader task.
func (m *copyMeter) toBootloader() {
	m.sample()
	m.mu.Lock()
	if m.bootloader {
		m.mu.Unlock()
		return
	}
	m.bootloader = true
	done := m.last
	m.mu.Unlock()
	if done > 0 {
		m.in.emit(Event{Kind: EventProgress, Step: StepCopy, Done: done, Total: done, Bytes: true})
	}
	m.in.emit(Event{Kind: EventDone, Step: StepCopy})
	m.in.begin(StepBootloader, 0)
}

// close stops sampling; ok tells whether bootc succeeded.
func (m *copyMeter) close(ok bool) {
	close(m.stop)
	<-m.stopped
	if !ok {
		return
	}
	// bootc without a bootloader message: the copy ends here, the
	// bootloader was part of it.
	m.toBootloader()
	m.in.emit(Event{Kind: EventDone, Step: StepBootloader})
}

// --- clock -------------------------------------------------------------------

// Ticker is a stoppable ticker; tests replace Installer.Ticker.
type Ticker interface {
	C() <-chan time.Time
	Stop()
}

type realTicker struct{ t *time.Ticker }

func (r realTicker) C() <-chan time.Time { return r.t.C }
func (r realTicker) Stop()               { r.t.Stop() }

func newRealTicker(d time.Duration) Ticker { return realTicker{time.NewTicker(d)} }

// --- tracking: speed and ETA ------------------------------------------------

// State of a task on the progress screen.
type State int

// Task states.
const (
	Pending State = iota
	Running
	Finished
	Failed
)

// Expected is how long each task takes when nothing better is known: the
// ETA of tasks that have not started or report no measurable progress. The
// copy takes almost all of the time and is measured by its speed; the
// others took under a second each on the QEMU install of the ISO (DIST-034)
// and get a few seconds for slower disks. The copy's own figure only
// matters before its speed is known (a 2.2 GB write at ~20 MB/s).
var Expected = map[StepID]time.Duration{
	StepPartition:  2 * time.Second,
	StepFormat:     2 * time.Second,
	StepCopy:       120 * time.Second,
	StepBootloader: 3 * time.Second,
	StepConfigure:  2 * time.Second,
	StepFinalize:   2 * time.Second,
}

// speedWindow is how far back the speed of a task is measured.
const speedWindow = 5 * time.Second

// Task is the state of one task as the progress screen draws it.
type Task struct {
	ID     StepID
	State  State
	Done   int64
	Total  int64
	Bytes  bool
	Detail string
	Error  string
	// Start and End, in milliseconds since the installation started.
	Start, End int64
	// Speed in units (bytes) per second over the last few seconds; 0 when
	// not measured yet.
	Speed float64

	samples []sample
}

type sample struct{ at, done int64 }

// Fraction is the share of the task done, or -1 when unknown.
func (t *Task) Fraction() float64 {
	switch {
	case t.State == Finished:
		return 1
	case t.Total <= 0:
		return -1
	}
	f := float64(t.Done) / float64(t.Total)
	return min(max(f, 0), 1)
}

// Elapsed is how long the task has run (until now, ms since the start).
func (t *Task) Elapsed(now int64) time.Duration {
	switch {
	case t.State == Pending:
		return 0
	case t.State != Running:
		now = t.End
	}
	return time.Duration(max(now-t.Start, 0)) * time.Millisecond
}

// ETA is the time left for a running task with known progress and speed.
func (t *Task) ETA() (time.Duration, bool) {
	if t.State != Running || t.Total <= 0 || t.Speed <= 0 {
		return 0, false
	}
	left := float64(max(t.Total-t.Done, 0)) / t.Speed
	return time.Duration(left * float64(time.Second)), true
}

// Tracker turns an event stream into the state of every task.
type Tracker struct {
	Tasks []*Task
	// Last is the time of the latest event (ms since the start).
	Last int64
}

// NewTracker starts with every task pending.
func NewTracker() *Tracker {
	t := &Tracker{}
	for _, s := range Steps {
		t.Tasks = append(t.Tasks, &Task{ID: s})
	}
	return t
}

// Task returns the task s (nil for an unknown one).
func (t *Tracker) Task(s StepID) *Task {
	for _, k := range t.Tasks {
		if k.ID == s {
			return k
		}
	}
	return nil
}

// Apply records one event.
func (t *Tracker) Apply(e Event) {
	k := t.Task(e.Step)
	if k == nil {
		return
	}
	t.Last = max(t.Last, e.At)
	switch e.Kind {
	case EventStart:
		*k = Task{ID: k.ID, State: Running, Start: e.At, Total: e.Total, Bytes: e.Bytes, Detail: e.Detail}
		k.samples = []sample{{e.At, 0}}
	case EventProgress:
		if k.State == Pending {
			k.State, k.Start = Running, e.At
		}
		k.Done, k.Total = e.Done, e.Total
		k.Bytes = k.Bytes || e.Bytes
		if e.Detail != "" {
			k.Detail = e.Detail
		}
		k.samples = append(k.samples, sample{e.At, e.Done})
		cut := 0
		for cut < len(k.samples)-2 && e.At-k.samples[cut+1].at >= speedWindow.Milliseconds() {
			cut++
		}
		k.samples = k.samples[cut:]
		first := k.samples[0]
		if dt := e.At - first.at; dt > 0 {
			k.Speed = float64(e.Done-first.done) * 1000 / float64(dt)
		}
	case EventDone:
		k.State, k.End = Finished, e.At
		if k.Total > 0 {
			k.Done = k.Total
		}
	case EventFail:
		k.State, k.End, k.Error = Failed, e.At, e.Error
	}
}

// Failed returns the failed task, or nil.
func (t *Tracker) Failed() *Task {
	for _, k := range t.Tasks {
		if k.State == Failed {
			return k
		}
	}
	return nil
}

// Remaining estimates the time left for the whole installation at now (ms
// since the start): the running task by its speed (or what is left of its
// expected time), the pending ones by their expected time.
func (t *Tracker) Remaining(now int64) time.Duration {
	var left time.Duration
	for _, k := range t.Tasks {
		exp := Expected[k.ID]
		switch k.State {
		case Pending:
			left += exp
		case Running:
			if eta, ok := k.ETA(); ok {
				// The speed was measured at the last sample.
				since := time.Duration(max(now-t.Last, 0)) * time.Millisecond
				left += max(eta-since, 0)
			} else if f := k.Fraction(); f > 0 {
				el := k.Elapsed(now)
				left += time.Duration(float64(el) * (1 - f) / f)
			} else {
				left += max(exp-k.Elapsed(now), time.Second)
			}
		}
	}
	return left
}

// Overall is the share of the whole installation done at now: the time
// spent over the time spent plus the time left, so the bar and the ETA
// always agree.
func (t *Tracker) Overall(now int64) float64 {
	all := true
	for _, k := range t.Tasks {
		if k.State != Finished {
			all = false
		}
	}
	if all {
		return 1
	}
	spent := time.Duration(max(now, 0)) * time.Millisecond
	left := t.Remaining(now)
	if spent+left <= 0 {
		return 0
	}
	return float64(spent) / float64(spent+left)
}

// --- checking a recording ---------------------------------------------------

// ETATolerance is how far the total time predicted halfway through may be
// from the real one in CheckRecording.
const ETATolerance = 0.25

// CheckRecording replays a recorded installation: it prints every task with
// its time and number of updates, and the total time the screen predicted at
// a quarter, half and three quarters of the way, and fails when a task did
// not finish, the copy did not move, or the prediction halfway was off by
// more than ETATolerance (the QEMU test of the ISO, DIST-034).
func CheckRecording(evs []Event, w io.Writer) error {
	if len(evs) == 0 {
		return fmt.Errorf("empty recording")
	}
	end := evs[len(evs)-1].At
	full := NewTracker()
	updates := map[StepID]int{}
	var problems []string
	last := map[StepID]int64{}
	for _, e := range evs {
		full.Apply(e)
		if e.Kind == EventProgress {
			updates[e.Step]++
			if e.Done < last[e.Step] {
				problems = append(problems, fmt.Sprintf("%s went back from %d to %d", e.Step, last[e.Step], e.Done))
			}
			last[e.Step] = e.Done
		}
	}
	for _, k := range full.Tasks {
		fmt.Fprintf(w, "%-10s %-8s %8s %4d updates", k.ID, stateName[k.State], k.Elapsed(end).Round(100*time.Millisecond), updates[k.ID])
		if k.Bytes {
			fmt.Fprintf(w, ", %d MB", k.Done/1e6)
		}
		fmt.Fprintln(w)
		if k.State != Finished {
			problems = append(problems, fmt.Sprintf("%s did not finish", k.ID))
		}
	}
	if updates[StepCopy] < 3 {
		problems = append(problems, fmt.Sprintf("the copy moved %d times", updates[StepCopy]))
	}
	total := time.Duration(end) * time.Millisecond
	fmt.Fprintf(w, "total      %s\n", total.Round(time.Second))
	for _, q := range []int64{25, 50, 75} {
		at := end * q / 100
		tr := NewTracker()
		for _, e := range evs {
			if e.At > at {
				break
			}
			tr.Apply(e)
		}
		pred := time.Duration(at)*time.Millisecond + tr.Remaining(at)
		off := float64(pred-total) / float64(total)
		fmt.Fprintf(w, "at %d%%     predicted %s (%+.0f%%)\n", q, pred.Round(time.Second), off*100)
		if q == 50 && (off > ETATolerance || off < -ETATolerance) {
			problems = append(problems, fmt.Sprintf("the prediction halfway was off by %+.0f%%", off*100))
		}
	}
	if len(problems) > 0 {
		return fmt.Errorf("progress check: %s", strings.Join(problems, "; "))
	}
	fmt.Fprintln(w, "progress check: OK")
	return nil
}

var stateName = map[State]string{Pending: "pending", Running: "running", Finished: "done", Failed: "failed"}
