# mouse testing suite

Measures what a pointing device actually does, rather than what it claims.

The interface is deliberately plain: black, grey and white, monospace, no
animation. Colour appears only to mean pass, warning or fail.

## Approach: one codebase, two backends

One Rust binary, built from one source tree, with the platform-specific
measurement layers selected at compile time. Neither platform is degraded to
keep them uniform, because the capability difference is real and the program
models it explicitly rather than papering over it.

Every measurement is taken at up to three levels, and the difference between
them is the point:

| level | macOS | Windows |
|---|---|---|
| **device** — reports as the OS receives them | `IOHIDDevice` input reports, timestamped by the driver | Raw Input (`WM_INPUT`), timestamped on arrival |
| **system** — after the OS turns reports into input events | system-wide mouse event stream | `WH_MOUSE_LL` low-level hook |
| **application** — what an ordinary program receives | events delivered to this window | events delivered to this window |

Two consequences are stated in the interface rather than hidden:

- The **device** level is per-device. The **system** level is not, on either
  platform: neither OS attributes a physical device to a system input event.
  With two pointing devices moving at once, the system level shows their sum.
- The device level is the rate *the OS receives*, not the rate the mouse sends.
  No unprivileged path exists below the OS class driver on either platform.
  Getting lower needs a signed kernel driver, which is a different product.

## What it measures

Eight sections, in the order they appear in the sidebar.

**DEVICE** enumerates pointing devices, shows their identifiers and connection
topology, reports exactly what access was obtained at each level, and describes
the host. It flags anything that would invalidate a timing measurement:
virtualisation, x64-on-ARM64 emulation, an active event tap filtering input, a
device behind USB hubs, a Bluetooth or internal transport whose reported
interval is a placeholder.

**POLLING** measures the real report rate at all three levels at once. A large
gap between the device figure and the application figure is an expected
finding, not a defect, and both are shown side by side rather than reconciled.
Dropped reports are distinguished from late ones by whether the gap is a clean
multiple of the interval. You can enter the configured rate for a
measured-against-claimed comparison.

**CLICKS** counts presses and releases per button, reports mismatches, and
detects contact bounce. Raw button identifiers are shown, so a button the OS
does not map is still visible. Distributions of press duration and inter-click
gap are drawn as histograms.

**CPS** is a timed click-rate test with a labelled mode selector (normal, drag,
butterfly, jitter). The label records technique and does not change the
measurement. Sustained rate leads; peak is secondary, because peak is noise.
Runs accumulate within the session.

**A/B** compares two firmware settings you can only change by hand. Described
in more detail below.

**SENSOR** verifies CPI against a distance you measure, detects drift and
jitter while stationary, detects angle snapping from a hand-drawn line, detects
motion smoothing, and finds the speed at which tracking degrades.

**SCROLL** counts detents in both directions and detects reversed and skipped
steps, on both the vertical wheel and a tilt wheel if there is one.

**SESSION** logs every event, exports the raw data and a readable summary, and
loads a previous export to re-analyse or compare against the current session.

### The A/B comparison

This is the part of the program that took the most care, because a person
trying to feel a difference between two mouse settings will find one whether or
not it is there. The run is arranged to make that as hard as possible.

Trials alternate in ABBA pairs rather than running all of A and then all of B,
so warm-up, fatigue and drift land on both conditions equally. Nothing about
the result is shown until the run finishes: no per-trial score, no running
total, no progress bar that hints at a number, because a person who knows they
are behind on B will try harder on the next B and turn the comparison into a
measure of effort.

The statistics assume nothing about the shape of the data. Wilcoxon signed-rank
on the paired differences is the primary test, Hodges-Lehmann is the effect
estimate, and the confidence interval is distribution-free with its **achieved**
coverage printed rather than a nominal 95% the discrete rank distribution
cannot deliver. Mann-Whitney appears as a secondary descriptive figure only,
since ignoring the pairing throws away the design.

A second variant counts how many of a fixed number of deliberately weak presses
each condition registers **at all**, rather than how fast you can click. That
is the measurement that separates a debounce setting which drops real inputs
from one which merely delays them, and rate testing cannot see it.

Raw per-trial data exports to CSV with the trial order preserved, so the
interleaving is auditable after the fact.

## Build

Needs Rust 1.92 or newer.

```
cargo build --release
```

### macOS (Apple Silicon)

```
sh scripts/install.sh
```

That builds the app, puts `Mouse Testing.app` into `/Applications`, and prints
what to do about permissions. It launches from Launchpad, Spotlight or the
Finder like anything else.

No `sudo`: `/Applications` is group-writable by `admin` and has no sticky bit,
so an ordinary copy is enough. Installing with `sudo` would leave a root-owned
bundle and force every future reinstall to need `sudo` as well.

No Gatekeeper prompt either. Quarantine is attached by whatever *downloaded* a
file, and nothing downloaded this one; a locally built binary carries only
`com.apple.provenance`, which Gatekeeper does not act on. (`spctl -a` still
reports `rejected` for an ad-hoc signature, but enforcement is gated on the
quarantine bit, which is absent.) If you ever zip the app and send it to another
Mac, that copy will be quarantined, and the fix there is
`xattr -dr com.apple.quarantine "/Applications/Mouse Testing.app"`.

To build without installing:

```
sh scripts/make-app-bundle.sh
open "target/Mouse Testing.app"
```

Use the bundle rather than `cargo run`. macOS attaches permission grants to the
"responsible process", which for a binary started from a terminal is the
terminal or editor, not the binary; the bundle launched with `open` is its own
responsible process and gets its own entry in System Settings.

### The icon

`scripts/icon_art.rs` **is** the icon: a list of drawing operations, rendered by
`scripts/icon.rs`, which is a standalone `rustc` file with no dependencies and
no place in the crate. `sh scripts/make-icon.sh` regenerates
`assets/AppIcon.icns` from it. The `.icns` is committed, so building and
installing never run that step; you only need it to change the mark.

The mark is a filled mouse showing only its wheel. Filled because a stroke is
the first thing downscaling destroys and this has to survive 16 pixels in a
Finder list; only the wheel, because drawing the button split as well put a
second mark directly above it, and below about 64 pixels the gap between them
closes and the pair reads as one long slot. The tile is rimmed in grey because a
flat black tile has no edge at all on a dark desktop, and because the app's own
interface draws exactly that rule around every group.

The tile matches Apple's own icon silhouette rather than approximating it.
Measured against six system icons, the shape is 80.5% of the canvas on a
0.0977 margin, and a superellipse exponent of 5.5 tracks their outline to within
1.3 pixels RMS at 256 across.

### Windows 10/11

```
cargo build --release
.\target\release\mouse-testing.exe
```

Build natively for the machine: `x86_64-pc-windows-msvc` on an Intel or AMD PC,
`aarch64-pc-windows-msvc` on an ARM PC. Do not ship or run a 32-bit build. The
app detects and warns about x64-on-ARM64 emulation, because emulated code adds
translation overhead that lands in the middle of every interval measurement.

## Permissions

No administrator or root privilege is required anywhere, and none is requested.

### macOS: Input Monitoring

Required for the **device** level only. Everything else, including the full
device list, identifiers, report descriptor and the host environment report,
works with no grant at all.

macOS gates Generic Desktop Mouse and Keyboard HID collections behind Input
Monitoring. Without it, `IOHIDDeviceOpen` returns `kIOReturnNotPermitted`. The
app detects this and says so with the exact error; it never reports zero.

To grant it:

1. **System Settings > Privacy & Security > Input Monitoring**
2. Switch on **Mouse Testing** (use **+** and pick
   `/Applications/Mouse Testing.app` if it is not listed)
3. **Quit and reopen the app.** macOS does not apply the grant to an
   already-running process.

The app has a button that opens that pane directly.

For an ad-hoc signed app the designated code requirement is *only* the code
hash: not the bundle identifier, not the path, not a certificate. Two things
follow, and both were measured rather than assumed.

The grant **follows the app** when you move it, because a code hash does not
depend on where the file lives. But macOS registers every copy under the same
bundle identifier, so leaving a second copy under `target/` makes the Settings
row ambiguous about which one it is describing.

Rebuilding **does not** cost you the grant unless the code actually changed.
Both the compile and the ad-hoc signing are deterministic: an unchanged rebuild
reproduced the same binary hash and the same bundle `CDHash` here, so the stored
requirement still matches. A real code change does invalidate it, and the repair
is `tccutil reset ListenEvent dev.mousetesting.suite` followed by re-granting.
(Changing the toolchain version or moving the project directory also changes the
hash, because debug info embeds absolute paths.)

### Windows: none

Raw Input and low-level mouse hooks need no privilege and no permission grant.

One Windows-specific hazard is surfaced rather than left to surprise you:
Windows silently removes a low-level hook whose procedure overruns
`LowLevelHooksTimeout`, and the symptom is a system level that goes quiet
rather than one that reports a failure. The app states the budget in force and
where that value came from.

## Testing a mouse you are not navigating with

Every timed test has a countdown before it starts, so you can put this
machine's own pointer down and pick up the mouse being measured. Captures also
run while the app is in the background, and on Windows the count of events that
arrived while the app was not in front is reported, because that is the proof
the arrangement works.

Events the operating system marks as synthesised by software are counted and
reported separately. Any of those inside a measurement window means the numbers
describe a program rather than the mouse.

## Exports

Exports go to `~/mouse-testing-exports`, and the full path is always shown,
because an app bundle launched from the Finder has no working directory you
could guess.

`session-<stamp>.csv` holds every captured event, with the device, the host,
the clock's resolution and read cost, and every environment warning that was
live at capture time carried in `#` comment lines at the top. That header is
the point: someone opening the file next month cannot re-derive that an event
tap was filtering input while it was recorded. The file loads back into the app
for re-analysis and comparison, and opens directly in a spreadsheet or with
`pandas.read_csv(path, comment='#')`.

`summary-<stamp>.txt` is the readable version, covering every section and
saying "not measured this session" for each one that was not run, because a
missing line and a clean result look identical otherwise.

Nothing is recorded that did not come from the mouse under test: no key
logging, no window titles, no cursor positions, no clipboard.

## Command line

```
mouse-testing --help             the list below, from the binary itself
mouse-testing                    run the interface
mouse-testing --dump             print the device and environment report, then exit
mouse-testing --dump-to FILE     the same, to a file
mouse-testing --section NAME     open on a named section
mouse-testing --screenshot FILE  render the window to a PNG and exit
mouse-testing --window WxH       window size, for screenshotting a tall section
mouse-testing --load-session FILE
                                 open a previous export
mouse-testing --capture-test SECS --out FILE
                                 capture unattended, export, verify the round
                                 trip, write a report and exit
mouse-testing --selftest-hid SECS
                                 exercise the HID capture path (macOS)
mouse-testing --request-access    ask macOS for Input Monitoring
mouse-testing --stats-check IN OUT
                                 run the statistics against a fixture
```

`--dump` is useful over SSH and for bug reports. `--screenshot` captures from
inside the process, so it needs no Screen Recording permission.

Three flags exist only to inspect result views without hardware, and touch
nothing in the normal path: `--ab-demo`, `--sensor-demo` (with
`--sensor-test NAME`) and `--scroll-demo`.

## What has been verified, and how

Honesty about this matters more than a green checkmark.

**macOS.** Built and run on macOS 15.6.1, Apple Silicon (M4). Device
enumeration, identifiers, IORegistry topology, permission probing and the
environment report are verified against real hardware. The capture engine — run
loop, callbacks, driver timestamps, ring buffer, teardown — is verified against
twelve real HID devices, which is possible because macOS gates only Mouse and
Keyboard collections, leaving every other HID device openable with no
permission at all. The export and reload path is verified end to end on 311
real captured events: identical metadata, identical events, and re-analysis
after reload matching the original to every printed digit.

**Windows.** Type-checked for both `x86_64-pc-windows-msvc` and
`aarch64-pc-windows-msvc`, including compile-time assertions on the Win32
struct layouts the decoder depends on. It has **not been run on Windows**; no
Windows machine was available. Treat the Windows path as unproven at runtime
until you exercise it.

**The device level has not been verified against a real mouse.** The machine
this was built on has no mouse attached, and the internal trackpad is behind
the Input Monitoring grant. Everything below the mouse-specific decode is
verified; the decode itself is verified against real report descriptors read
off this machine, but not against a real mouse's report stream.

**Statistics.** Verified against scipy and numpy independently rather than
trusted: 19 cases, worst absolute error 3.6e-15 (`scripts/verify_stats.py`).
Four real bugs were found and fixed in the process, including two that took
down the interface on an ordinary long run.

**Analysis.** Every detector is checked against a simulator with known ground
truth, because the alternative is asserting that a detector agrees with itself.
The simulator models the chain in the order the hardware has it: hand
trajectory, then sensor noise, then firmware angle-snap, then firmware
smoothing, then the residual-accumulator quantiser, then report-field clipping,
then dropped polling slots, then timestamp jitter. That order is load-bearing.
Put the snap before the noise and the anisotropy detector reads 1.00 for clean
and snapped alike, having exactly zero power while appearing to work.

Each detector is asserted in both directions, and the first matters more: a
false alarm on working hardware is what makes a measurement tool useless. CPI
is recovered unbiased to 0.5% and a 10% error caught 28 times in 30; drift is
separated from jitter with at most 2 false calls in 60; snapping fires on 0 of
40 clean hand-drawn strokes and catches 30 of 40 fully snapped ones; smoothing
time constants are recovered within 25%; every common scroll wheel shape has
its detent size recovered exactly, with 3% reversals and 8% skips each caught
in 36 of 40 runs and a fast flick never called a skip.

**Interface.** Layout invariants are enforced by tests that run headlessly,
including that a numeric readout's width does not change with its value, that a
grouping box does not resize when the text inside it changes, that every text
style is monospace, that nothing is rounded, and that colour is used only for
state.

`cargo test` runs all 95. `cargo build` is clean of warnings on macOS and on
both Windows targets.

## Where the measurement refuses to answer

Six of the detectors will return **inconclusive** rather than a number when the
protocol was not met: a snapping stroke too slow or too short, a smoothing
glide too fast, too few judgeable polling intervals, a free-spinning scroll
wheel with no detents to count, spam-clicking during a debounce test, and a
tracking test where the sensor never actually failed.

Inconclusive deliberately outranks pass, so it cannot be masked. A refusal to
answer is a better result than an answer computed from a stroke that could not
carry the information.

Two more cases disable themselves visibly. The angle-snapping test is a ratio
against the sensor's own noise, so on a sensor too quiet to have any it reports
itself not applicable rather than passing you. The whole sensor section
requires signed per-axis counts, and says so plainly when the device level
cannot supply them, rather than analysing zeros.

## Where intent was guessed

Three places, stated so they can be corrected.

The **weak-input A/B variant** is described in the requirements as "counting
total actuations under deliberately weak input". A count needs a denominator to
mean anything, so it is implemented as a fixed number of attempts per trial,
with the instruction to press as lightly as possible while still trying to
actuate. The score is how many of those attempts the mouse registered.

**"Both directions"** for the scroll wheel is read as up and down. The tilt
wheel is analysed as a separate encoder when it is present, on the grounds that
it fails independently, but it is not what the requirement meant.

**Contact bounce thresholds** are anchored on Omron D2F/D2FC datasheet figures
(5 ms typical, 10 ms worst case) and community measurements of butterfly and
jitter clicking rates, recalled rather than fetched. The margins are wide on
both sides, and the doublet rule scales against the clicker's own rhythm rather
than a fixed number, but the constants themselves are worth checking before
anyone quotes them.

## Non-goals

No RGB control, macro recording, writing configuration to the mouse, cloud
features, accounts, or auto-update. Measurement only.
