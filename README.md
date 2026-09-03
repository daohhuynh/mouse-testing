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
| **device**: reports as the OS receives them | `IOHIDDevice` input reports, timestamped by the driver | Raw Input (`WM_INPUT`), timestamped on arrival |
| **system**: after the OS turns reports into input events | system-wide mouse event stream | `WH_MOUSE_LL` low-level hook |
| **application**: what an ordinary program receives | events delivered to this window | events delivered to this window |

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

## Using it, and what the results mean

Written for someone who just wants to know whether their mouse is working
properly. It assumes you know nothing about any of this. Install the app first
(see Build, below), then come back here.

One word first, because the whole app is built on it. A mouse works by sending
the computer a stream of short messages, one every few thousandths of a second,
each saying how far it just moved and which buttons are down. This guide calls
one of those a **report**. Most of what follows is about counting them, timing
them, and noticing when one goes missing.

### The word in brackets

Every verdict line carries a word in brackets, and there the colour only
repeats what the word already says.

| tag | colour | what it means |
|---|---|---|
| `[PASS]` | green | Measured, and nothing wrong was found. |
| `[WARN]` | amber | Something is a little off, or the app cannot check yet. |
| `[FAIL]` | red | Measured, and it is wrong. |
| `[N/A ]` | grey | Not measured, or not measurable. |
| `[----]` | white | A fact, with no judgement attached. |

Some numbers are a verdict on their own, and turn amber or red the moment they
stop being 0, with no word beside them. So if you cannot tell red from green,
read the bracketed lines and the grey notes rather than the colour of a figure.

`[N/A ]` is the one people misread. It does not mean "fine". It means the app
declined to answer. The app treats that refusal as more serious than any pass,
so a good result somewhere else never hides it. Read the grey note underneath.
Usually it says what to do differently, and doing that turns it into a real
answer. Sometimes it says the thing cannot be measured at all, and then there
is nothing for you to do.

### Start here, every time

1. Open the app. It opens on **DEVICE**.
2. Under "attached pointing devices", click the mouse you want to test. If it
   is not listed, plug it in and press **refresh**. Nothing on this screen
   updates on its own.
3. Check for an amber line reading `This collection is not a Generic Desktop
   Mouse, so it is not the pointing interface.` Some mice list themselves more
   than once, and only one of those entries is the actual mouse. If you see
   that line, go back and pick a different row for the same device.
4. Look at the connection line. `USB, direct to a port on the host controller`
   is the good case: the mouse is plugged straight into the computer with
   nothing in between. `behind 1 external hub(s)` or `Bluetooth` are warnings,
   because a dock, a hub or a radio link adds delays of its own that will land
   in every timing measurement you take afterwards. The rest of that line is
   the speed of the connection, and if it says `low`, the connection itself
   caps the mouse at 100 reports a second however you have the mouse set.
5. On macOS, look at "access obtained". If the **device** line is red, the app
   cannot read the mouse directly, and SENSOR, SCROLL and the verdict in
   POLLING will not work at all. Grant Input Monitoring (see Permissions,
   below), then **quit the app and open it again**. macOS does not apply the
   grant to a program that is already running.

   If the switch is **already on** and the line is still red, the saved
   permission belongs to an older build of the app. macOS identifies this app
   by a hash of its code, so rebuilding it stops the saved permission matching,
   and since nothing cancelled that permission the switch stays on and does
   nothing. Clearing it is the whole fix, and the surest way is to ask for it:

       sh scripts/install.sh --reset-permission

   A plain reinstall does the same thing whenever it replaces a different
   build, but it cannot help here, because the copy you have installed is the
   build being refused and an unchanged rebuild has nothing to notice. Clearing
   the permission removes the row, so afterwards reopen the app and press **ask
   macOS now**, or add it again with **+** in Settings.

   Two more things about that screen catch people out. The **system** line
   always reads amber `[WARN]`. The app is written that way, and it is not a
   problem with your machine. And a green **device** line only means the app
   could open *some* mouse, not necessarily the one you picked, which is why
   step 3 matters.

   On Windows there is no permission to grant and nothing to do here.
6. Read "measurement validity" at the bottom. `Nothing detected that would
   invalidate timing measurements` means you are clear to go. If it lists
   anything in amber or red, fix that first; each line names what it found.
   Those warnings are the difference between measuring your mouse and measuring
   your dock, your Bluetooth link, your battery saver, or some other program
   sitting between the mouse and you. Lines in white are only describing your
   machine, and the note under each one says whether it actually spoils
   anything.

One thing on this screen is not a measurement. On macOS, "advertised interval"
is the rate the mouse *claims*, read off the mouse itself. It is written as the
gap between reports in millionths of a second, with the same figure converted
to hertz beside it when the app trusts the number. Hz just means times per
second, so 1000 Hz is a thousand reports a second, which is the same thing as
1000 millionths of a second between them. A mouse that says 1000 Hz will say it
whether or not it delivers, and on Bluetooth or a built-in trackpad the app
labels the figure a placeholder rather than a claim. On Windows the app does
not read it at all and the line always says "not reported". POLLING is what
turns any of this into a real number.

### If you only want to know whether your mouse is failing

Four tests, about twenty minutes, in this order.

1. **CLICKS**, for a button that double-clicks on its own. This is by far the
   most common way a mouse dies.
2. **SCROLL**, for a wheel that jumps backwards or skips.
3. **POLLING**, for stutter and lost reports.
4. **SENSOR**, for a pointer that drifts, or a sensitivity setting that is
   lying to you.

Everything else is for comparing two mice, or two settings, or the same mouse
before and after something changed.

### CLICKS: is a button registering clicks you never made?

**What it answers.** Whether one physical press is being recorded as two. That
is the file dropped in the wrong folder, the thing that opens twice, the shot
that fires twice.

**What you do.** Start the run with the `F5` key or the space bar while the app
is the window in front, or press **start now**. Then click each button in turn:
left, right, wheel, side buttons.

If you did use the button, move the pointer away from it before you start
clicking. **start now** turns into **stop** in the same place, so your next
click there would end the run. `F5` and the space bar avoid that entirely.

**Click deliberately, about once a second. Do not click fast.** This is the
whole trick of the test. A worn switch bounces and produces two clicks a few
thousandths of a second apart, and if you are spam-clicking, your own gaps land
in the same range and the two become impossible to tell apart. If your typical
gap drops below 100 ms the app notices, and reports the borderline gaps as
`[N/A ]` rather than as a warning. Gaps too short for any hand still count
against the mouse, so click slowly anyway.

Give each button at least 20 clicks, which is the fewest the app will judge at
all, and about a hundred before you trust a pass. A switch that misfires now
and then can easily behave itself for twenty clicks.

**What the results mean.**

- `[PASS] No contact bounce detected.` Contact bounce is the name for a worn
  switch turning one press into two clicks. None was found.
- `[FAIL]` with a doublet reported. A doublet is one press that arrived as two
  clicks, closer together than any hand can click. The switch is worn. This
  fails immediately, at any number of clicks, because one genuine doublet is
  already proof.
- `[FAIL]` on the bounce rate. At least one press in every hundred started less
  than 15 thousandths of a second after the button came back up. No hand clicks
  that fast, so there are too many of these to be you. The switch is worn.
- `[WARN]` Something faster than a deliberate hand: either two clicks too close
  together, or one press held far too briefly. Too rare to condemn the switch
  outright. Read the counts, then run it again with more clicks.
- `[WARN] Presses and releases did not pair up.` Usually you were holding a
  button when the run started, or an event was lost. Run it again.
- `[N/A ]` under 20 presses, for a good button and a bad one alike. The one
  exception is an outright doublet, which fails at any number of clicks.

Buttons are listed by raw hardware number, not by name, so a button your system
does not recognise still shows up. To find out which is which, start a run,
press one button, and watch which block's count goes up.

### SCROLL: is the wheel counting your clicks correctly?

**What it answers.** Whether a click of the wheel ever goes the wrong way, or
gets counted twice. That is the page that jumps back a line while you scroll
down, and this test is how you find out whether the wheel is to blame.

**What you do.** Pick how long to record under "capture for": 10, 20, 40 or
60 seconds, starting at 20. Pick 40 or 60 if you want the fuller answer below,
because the target there does not fit into 20 seconds of ordinary scrolling.
Press **start**, wait out the countdown, then scroll at your normal reading
pace.

**Do not flick or spin the wheel.** Scroll roughly the same amount up as down,
because the two directions are counted separately and a wheel can be fine one
way and broken the other. Thirty clicks in each direction is enough to catch an
obvious fault; about 150 in total if you want to catch a wheel that only
misbehaves 3% of the time. If your wheel tilts sideways, tilt it too, since
that is a separate mechanism with its own result.

Use the **start** button here, not `F5` or the space bar. Those two toggle the
underlying recording from anywhere in the app, and pressing either one during a
scroll run tears down the capture the run is reading from.

**What the results mean.**

- **reversed steps** should be 0. It turns red the moment it is not. A reversed
  step is one wheel click sent the wrong way while the clicks either side of it
  went the right way. That is the part inside the wheel that counts your clicks
  getting one wrong.
- **skipped steps** should be 0. It turns amber the moment it is not. A skipped
  step is one wheel click that arrived as two, so the page moves twice as far
  as you asked.
- The verdict comes from how often these happen, not from the raw counts, so a
  long run is judged the same way as a short one. Reversals: under 0.5% passes,
  0.5% and up is a warning, 2% and up is a failure. Skips: under 1% passes, 1%
  and up is a warning, 5% and up is a failure.

Two results that look like problems and are not:

- `[N/A ] fewer than 10 scroll clusters; scroll more.` The app did not see
  enough of what it calls steps, which at an ordinary pace is about ten wheel
  clicks. On screen that figure is "steps recorded". Scroll more, and check
  that the mouse you scrolled is the one highlighted on DEVICE and that its
  access line is not red.
- **No tilt wheel result.** Most mice have no tilt wheel, and one that does may
  simply not have been tilted. This is not a finding either way.

Some wheels spin freely instead of clicking from notch to notch. The app calls
those continuous and refuses to judge them, because this test works by counting
wheel clicks and there are none to count. That is not a clean bill of health.
When a wheel is called continuous the reversed and skipped counts are still
drawn, and still coloured, but they mean nothing, which is why the verdict
beside them is `[N/A ]`. A trackpad is a different case again: it sends no
wheel clicks at all, so you get the "scroll more" result instead.

### POLLING: how often does the mouse actually report?

**What it answers.** How many reports a second your mouse really sends, and
whether any of them went missing or arrived late. Missing reports are the small
stutters and the flick that lands slightly off target.

On macOS the verdict here needs Input Monitoring, like SENSOR and SCROLL.
Without it the device row is blocked and no verdict appears, though the system
and app rows still fill in: the way this app watches system-wide mouse events
on macOS needs no permission at all.

**What you do.** Type the rate your mouse is set to, in reports per second,
into the "configured rate" box. `1000` is a common one, and you will find it in
the mouse's own configuration software. You have to type it because the
"advertised interval" on the DEVICE screen is only what the connection
advertises, and on most mice that figure does not follow the rate you actually
picked. Neither operating system will tell the app the real setting.

Then press **start now**, or a delayed start if you need both hands free, and
**swipe the mouse hard and keep swiping**.

Moving gently is the usual reason this test gives no answer. A mouse sends
nothing when it has nothing to send, and a silent moment cannot be told apart
from a lost report, so those moments are thrown away rather than counted
against you. Keep going until "intervals judged" reaches 200 and turns green.
An interval is the gap between one report and the next, so that number is
counting usable gaps, not seconds. Below 200 the app gives no verdict at all.

**What the results mean.**

- **nominal rate** is the figure to read, despite the name. It is taken from
  the moments you were actually moving, so it is the mouse's real heartbeat.
- **sustained rate** runs from your first report to your last and counts every
  moment in between, including the ones where you were not moving, so it reads
  low unless you swiped continuously.
- `[PASS]` Essentially nothing went missing or arrived late.
- `[WARN]` At least 0.1% of reports dropped, or 0.5% late. At 1000 reports a
  second, 0.1% is about one lost per second, which nobody can feel.
- `[FAIL]` At least 1% dropped or 2% late. At 1000 reports a second, 1% is ten
  lost every second, which is the stutter people actually notice.
- `[N/A ] Not enough information for a verdict.` Read the note. Usually it is
  "swipe faster and for longer". Sometimes the gaps were too uneven to tell a
  late report from a missing one, which is common over Bluetooth and on a busy
  machine. Close what you can, plug in by cable, and run it again.

If the drop rate is only just over the line, between 0.1% and 0.2%, the app
says so. Good hardware wanders that much between runs and so does the app's own
measuring, so run it again before concluding anything.

**Three rows, and why they disagree.** The **device** row is your mouse. The
**system** row is the sum of every pointing device you touch, so nudging the
laptop trackpad shows up there. The **app** row usually looks far worse than
the other two, and that is the correct answer rather than a fault: an ordinary
program only receives pointer updates as fast as it redraws, and the operating
system throws the rest away. It also counts only while the pointer is over this
app's own window. That gap is a large part of why this app exists.

The chart underneath has no numbers along its edges, and does not need them. It
sorts the gaps between reports by size and shows you the shape, with marked
guides at one, two and three times the expected gap. One tall narrow column at
the first guide means the gaps were all the same, which is a healthy heartbeat.
Columns at the second and third guides are reports that went missing.

### SENSOR: is the mouse reporting your hand honestly?

SENSOR and SCROLL are the two sections that need raw access to the mouse. If
macOS Input Monitoring is not granted, both say so and measure nothing.

There are five tests. Each has its own on-screen instructions, its own fixed
recording length, and a countdown at the start so you have time to let go of
whatever you used to press the button and get your hand on the mouse you are
testing. Leave the countdown at 3 seconds or more. At 0 seconds, the movement
you make reaching for the mouse ends up inside the recording.

"Configured CPI" means counts per inch: how much movement the mouse reports for
an inch of hand travel. Most mouse software calls it DPI. Type what the mouse
is **currently set to**, not what the box says and not what the marketing page
says. If the mouse is set to 800 and you type 1600, the app will report a
perfectly good mouse as 50% off.

**counts per inch** checks whether that sensitivity number is true. Put two
marks on the desk and measure the distance between them. The further apart the
better, because a small slip over 8 inches matters far less than the same slip
over 2. Type that distance into "distance swiped", using the button beside it
to switch between inches and mm; the test will not start until both that box
and "configured CPI" have a number in them. Line the mouse up on the first
mark, press start, wait out the countdown, then swipe smoothly and fairly
slowly to the second mark. Do it several times; the app takes the middle value.

Within about 2% of what you typed is a pass, 2% to 5% is a warning, and over 5%
means the setting on your mouse is not telling the truth. That does not make
the mouse unusable. It means it is more or less sensitive than it claims, which
matters mostly when you are trying to match two mice or move a setting between
them.

The app refuses to answer, rather than guess, if your swipe curved, wandered
off the straight line, was too short, or was fast enough that the sensor might
have been losing counts. The note says which.

**drift and jitter** are measured with your hand off the mouse entirely. Drift
is the pointer wandering off in one direction on its own, which is the
crosshair that slides off a target while you hold still. Jitter is the pointer
shimmering in place and going nowhere. Both are counted in the smallest step
the mouse can report, called a count. The app only calls something drift once
the movement is consistently one way, and from there 1 count per second is
where the warning starts and 5 is a failure. For jitter the lines are 5 and 20.

**angle snapping** detects the mouse's own built-in software quietly
straightening your lines, so a nearly straight stroke comes out perfectly
straight. Draw a freehand diagonal. Do not use a ruler, and do not draw
straight up or straight across, because a line you drew perfectly horizontally
cannot be told apart from one the mouse snapped to horizontal.

**motion smoothing** detects that same built-in software averaging your
movement out to look smooth. That is the "floaty" feel some mice have, where
the pointer keeps sliding for a moment after your hand has stopped. It needs an
abrupt stop to measure, so glide the mouse at a moderate pace into the edge of
the mousepad or a book laid on the desk, and let that stop it dead. A hand
slowing down on its own looks the same as a smoothed mouse, which is why the
stop has to be sudden.

**maximum tracking speed** finds the speed at which the sensor stops keeping
up, which is the fast flick that lands somewhere you did not aim. Swipe as fast
as you can. If you never actually outran the sensor, the app reports the
fastest speed you reached as a floor rather than a limit, and only calls that a
pass if you got past 200 inches per second. Below that it says the test was
inconclusive, which is a comment on your swipe and not on the mouse.

### CPS: how fast can you click?

This one scores you, not your mouse. There is no pass or fail anywhere in it
and no target number: a clicks-per-second figure only means something next to
another figure from the same person, the same duration and the same technique.

**What you do.** Pick a duration: 5, 10, 30 or 60 seconds. Pick which button
counts, or leave it on "any". Pick the technique label that matches how you are
about to click. Then press **start**, or "after" and a delay if you need a
moment, and click until the timer runs out. The run ends and scores itself.
Runs pile up in a table, so you can do several and compare.

**sustained** is the honest number. It counts every second of the run,
including any you spent resting, so a pause in the middle drags it down on
purpose. **peak** is the busiest single second, which on a short run is mostly
luck about where that second fell.

The technique buttons (normal, drag click, butterfly, jitter) are labels only.
They are stored with the result so you can compare like with like, and change
nothing about what is counted. Jitter here is the name of a clicking style and
has nothing to do with the sensor test above.

One trap: a switch with contact bounce *raises* your score here, because every
spurious extra click is counted. Run CLICKS first.

### A/B: did that setting actually help?

**What it answers.** You changed a setting on the mouse: how often it reports,
or its debounce time, which is how long the mouse ignores a button after a
press so that one press cannot count as two, or a click mode. You cannot tell
whether the change really helped or whether you just had a good run. This is
the part that answers that honestly.

**What you do.** Name the two settings, for example `debounce 4 ms` and
`debounce 0 ms`. Under "what to measure", pick one of two:

- **click rate** asks which setting lets you click faster. Click as fast as you
  can keep up for the whole trial; being consistent beats a burst at the start.
- **actuations under weak input** asks which setting misses fewer of your
  gentle presses. Press *as lightly as you can* while still meaning to click,
  at a steady rhythm of about two presses a second. The rhythm matters as much
  as the lightness, because the app counts how many presses registered and
  never how many you attempted. Pressing harder to score better destroys the
  measurement.

Then pick how long each trial lasts and how many pairs to run, and press
**start run**. For every trial the app tells you which setting to put the mouse
on and waits. Change it, press **ready**, and a short countdown runs before it
records. Nothing moves along until you press **ready**, so take your time.

Two things are deliberately awkward, and both are the point:

- **You are told nothing until the very end.** No running score, no progress
  towards a number. If you knew you were behind on B, you would try harder on
  the next B, and the test would end up measuring your effort instead of your
  mouse.
- **The order is balanced, not a plain A B A B.** A pair is two trials, one of
  each setting, and the pairs take turns going first: A B, then B A, then A B,
  then B A. That is the ABBA arrangement described further up. It puts warm-up,
  fatigue and boredom equally on both settings, and it means the same setting
  comes up twice in a row at each boundary, which is intended.

**What the results mean.**

- **median difference** is the headline: how much better one setting was in a
  typical pair.
- **interval low / interval high** is the range the true difference is probably
  inside, given what this run saw. If zero sits inside that range, no
  difference at all is still a real possibility, and this run cannot tell the
  two settings apart.
- **interval covers** is how confident that range really is. The app prints the
  coverage it actually achieved rather than a flattering 95%, for the reason
  given under The A/B comparison above.
- **p value** answers one question: if the two settings were really identical,
  how often would a run like this show a gap this big by luck alone? 0.05 means
  one run in twenty, and it is the only threshold the app applies. Below it you
  get `X beat Y by N at the median`. At or above it you get `No difference this
  run can distinguish`, which is not proof they are identical. It means any
  real difference is smaller than this run could see.

Three ways to fool yourself, all worth avoiding:

- Nothing checks that you actually changed the setting when asked. Forget one
  changeover and that pair is quietly comparing a setting against itself.
- Do not keep adding pairs until the answer looks good. Every time you look at
  the result and decide to carry on, you give luck another chance to hand you a
  winner that is not real. Checking after every pair from 6 up to 20 turns a 5%
  chance of a false winner into 16%, and even three looks makes it 10%. Choose
  the number of pairs before you start.
- Do not switch hands, chairs or desks partway through. The test can cancel out
  a steady drift, but not a change you make to yourself.

### SESSION: saving what you measured

This screen runs no test of its own. It reports what has been recorded, saves
it, and puts an older saved recording beside the current one.

Two rules, and both bite:

- **Export before you press stop.** Stopping sets the recording clock back to
  zero. Every event and every verdict is still written afterwards, but both
  files will give the length of the session as 0 seconds.
- **Starting a new recording wipes the previous one.** The captured events and
  the POLLING result are gone if you did not export them. On macOS the button
  history goes with them; on Windows it carries over, so use "clear button
  history" on CLICKS if you want a fresh count. The SCROLL, SENSOR, CPS and A/B
  verdicts stay on screen either way, so a summary exported later can mix those
  older verdicts with a newer recording.

Remember that `F5` and the space bar toggle the recording from anywhere in the
app, including this screen. One tap stops it, which is what zeroes the duration
in the export, and the next tap starts a fresh one, which wipes the recording.

**export raw data and summary** writes two files to `~/mouse-testing-exports`.
The `.csv` is every event, and it opens in a spreadsheet. The `.txt` is the
readable version of everything measured, and it says "not measured this
session" for anything you did not run, so a missing test and a clean result
never look alike.

Three numbers here are worth checking before you trust anything else:

- **buffer losses** should be 0. Anything above 0 means events arrived faster
  than the app could store them, so the recording has holes in it. That is not
  your mouse's fault, but any rate worked out across a hole is wrong. Close
  whatever else is busy on the machine and record again.
- **synthesised by software** counts movements and clicks that a program on
  your computer made up rather than your mouse sending them. The line only
  appears when there is something to report, drawn in red, so on a clean run
  you will not see it at all. If you do see it, find the macro tool, remote
  control app or automation utility responsible, close it, and record again.
  This counter is Windows only, so its absence on macOS proves nothing.
- **device / system / app level** counts tell you which of the three ways of
  watching the mouse are collecting anything. They are the same three described
  under POLLING above. Zero at the device level while you are moving the mouse
  means that level is not collecting: usually no mouse picked on DEVICE, the
  wrong mouse picked, or a missing permission.

To compare against a previous run, load the older `session-....csv` and pick a
level. Load the `.csv` rather than the `.txt` written beside it; the summary
file is for reading, and the app will refuse it. What you get back is a timing
comparison: event counts, rates, dropped reports, motion, interval figures and
the number of button transitions, which counts every press and every release
separately. The old CLICKS, SCROLL and SENSOR verdicts are not shown again.
The app will tell you if the two recordings are of different mice, which is the
usual reason two sets of numbers refuse to line up.

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

eframe substitutes its own icon when the caller supplies none and pushes it onto
the running application, which on macOS overrides the bundle icon: the Finder
showed the real mark while the Dock and Command-Tab showed egui's hexagon. So
the icon is set explicitly. macOS gets a deliberately EMPTY one, because eframe
discards an icon equal to the default and then makes no call at all, leaving the
bundle's ten separately rendered sizes in charge. Windows has no bundle to carry
an `.icns`, so there a 128 px bitmap is compiled into the binary;
`make-icon.sh` writes both from the same geometry in the same run.

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
requirement still matches.

A real code change does invalidate it, and the failure is quiet in the worst
way: **the switch in System Settings stays ON.** Nothing revoked the
authorisation, so the row keeps its state; only the stored requirement stopped
matching. The app is refused while looking granted, and the refusal is
identical to never having been granted, so nothing on either side can tell you
which one you are looking at. `scripts/install.sh` therefore compares the
outgoing bundle's `CDHash` with the incoming one and clears the grant itself
when they differ, since a grant pinned to a hash that no longer exists is
already dead and the row that survives it only misleads. A grant that still
matches is left alone, which is the point of the build being reproducible.

That comparison cannot help anyone already stuck, though: their installed copy
*is* the build being refused, so the reinstall is byte-identical and there is
nothing to notice. For that case, and for a grant made against a bundle under
`target/` before the first install, ask for the reset directly with
`sh scripts/install.sh --reset-permission`, or run
`tccutil reset ListenEvent dev.mousetesting.suite` by hand. Either removes the
row, so re-granting means letting the app ask again or adding it with **+**.
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
environment report are verified against real hardware. The capture engine (run
loop, callbacks, driver timestamps, ring buffer, teardown) is verified against
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
