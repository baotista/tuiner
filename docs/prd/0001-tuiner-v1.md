# PRD 0001 — Tuiner v1

Terminology follows [`CONTEXT.md`](../../CONTEXT.md); capitalised terms are glossary terms and should
be used verbatim in code and commits. Decisions already recorded in [`docs/adr/`](../adr/) are binding
and are referenced rather than re-argued.

## Problem Statement

A guitarist or bassist working in a terminal has no way to tune without reaching for a phone app, a
clip-on tuner, or a browser tab. Those alternatives share three failures. They do not know which
instrument is in your hands, so they report a bare note name and leave you to work out whether that
note is the one you wanted. They stop being useful precisely at the end of the job — the last few cents,
where a needle sits stubbornly near the middle of a scale too coarse to show the difference between one
cent and three. And they cannot see the audio interface the instrument is actually plugged into, so a
player with a soundcard is pushed back to a laptop microphone and whatever the room is doing.

The player wants to tune from the terminal they are already in, against the Tuning they are actually
using, with enough precision that a chord rings true afterwards.

## Solution

A terminal application that listens to a chosen Input Channel, works out the sounding Pitch, and shows
which way to turn the Peg.

It runs in two Modes. In **Guided Mode** the player picks an instrument and Tuning up front; the app
matches the sounding Pitch to the nearest String in that Tuning and reports Deviation against that
String's Target Pitch, so Strings may be played in any order and the display always names the String
rather than an abstract note. In **Chromatic Mode** there is no Tuning and the app reports Deviation
against the nearest Note.

Precision is carried by a **Strobe** rather than a needle. Because a Strobe encodes Deviation as rate of
apparent motion instead of position on a scale, it has no floor: half a cent flat is visible as motion
too slow to complete, which no scale that fits in a terminal can show. A coarse bar gets you to the
right note; the Strobe finishes the job.

On first run the player chooses an Input Device and Input Channel with live Level meters, so it is
obvious which jack the instrument is in. That choice persists, and later runs go straight to tuning.

## User Stories

### Choosing an input

1. As a player, I want to see every Input Device the system exposes, so that I can pick the one my instrument is connected to.
2. As a player with a USB audio interface, I want to choose which Input Channel to listen to, so that the app hears the jack my instrument is in rather than averaging it with an empty one.
3. As a player with a single-channel microphone, I want the channel step skipped entirely, so that the simple case stays simple.
4. As a player unsure which jack I used, I want a live Level meter beside each Input Channel, so that I can see which one responds when I play.
5. As a returning player, I want the app to remember my Input Device and Input Channel, so that daily use costs no keystrokes.
6. As a player whose interface is unplugged, I want to be told the remembered Input Device is missing and be offered the picker, so that the app never silently listens to something else.
7. As a player, I want to reopen the input picker at any time with a keypress, so that I can switch instruments mid-session.
8. As a player with no capture devices at all, I want a plain message and a clean exit, so that I am not left staring at an empty interface.
9. As a player scripting a setup, I want to select the Input Device and Input Channel from the command line, so that I can skip the picker entirely.
10. As a player, I want the app to work at whatever sample rate my device reports, so that a 44.1 kHz interface behaves identically to a 48 kHz one.

### Tuning in Guided Mode

11. As a guitarist, I want to choose from Standard, D Standard, Open C and DADGAD, so that the app knows the Target Pitch of every String I am tuning.
12. As a bassist, I want a four-String Bass Standard Tuning, so that the app is not pretending I have six Strings.
13. As a player, I want to cycle Tuning with a keypress, so that changing Tuning does not mean restarting.
14. As a player, I want to play my Strings in any order and have the app work out which one I am on, so that I am not pressing a key before every String.
15. As a player, I want the app to name the String and its Note together, so that I never have to translate a note name into a String position.
16. As a player, I want to see at a glance which Strings are already in tune and which I have not touched, so that I know how much is left.
17. As a player, I want the app to tell me to tighten or loosen, so that I do not have to remember which direction is sharp.
18. As a player, I want to be told to tighten or loosen and never to turn clockwise, because which rotation tightens depends on my machine heads and which side of the Headstock the Peg is on.
19. As a player, I want a String declared in tune only within the In-Tune Tolerance of ±3 cents, so that a chord rings true afterwards.
20. As a player tuning a badly detuned String, I want the app to refuse to guess when the Pitch falls outside every Capture Range, so that the arrow never reverses direction while I turn the Peg one way.
21. As a player in DADGAD, I want the app to handle the G3 and A3 Strings being only a whole tone apart, so that it does not flip between them.
22. As a player fitting a fresh String, I want to lock the app onto one String with a number key, so that it keeps guiding me even though the String starts far too flat to be matched.
23. As a player, I want to release a String Lock with the same key, so that the app returns to matching automatically.
24. As a bassist, I want the low E String tracked without octave errors, so that the app does not read E2 while I am playing E1.

### Tuning in Chromatic Mode

25. As a player with an instrument the presets do not cover, I want a Chromatic Mode, so that the app is still useful.
26. As a player, I want to toggle between Guided and Chromatic Mode with a keypress, so that switching costs nothing.
27. As a player in Chromatic Mode, I want the nearest Note and its Deviation, so that I can tune anything by ear-free reference.
28. As a player, I want Chromatic Mode to cover B0 to E6, so that a five-String bass low B and a guitar top E both read.

### Reading the display

29. As a player, I want a Strobe that freezes when I am in tune, so that the last cent is visible as motion rather than as a marker I cannot resolve.
30. As a player, I want the Strobe to drift one way when flat and the other when sharp, so that direction is unambiguous without reading text.
31. As a player, I want a coarse bar covering ±50 cents, so that I can find the right note before worrying about precision.
32. As a player, I want a Deviation Trail over the last several seconds, so that I can see whether the String is settling, overshooting, or drifting back out.
33. As a player, I want a Headstock showing my Pegs with each one tinted by its String's state, so that I can read progress as a picture rather than as a list.
34. As a player, I want the exact sounding frequency in hertz and the Deviation in cents shown numerically, so that I can see precise values when I want them.
35. As a colour-blind player, I want blue for flat and orange for sharp rather than green and red, so that the primary signal is legible to me.
36. As a player, I want direction encoded in hue and magnitude encoded in bar position and Strobe rate, so that no single piece of information depends on colour alone.
37. As a player on a light-background terminal, I want the palette to remain legible, so that the app is usable outside a dark theme.
38. As a player on a small terminal, I want panels dropped in a sensible order rather than a broken layout, so that the app degrades instead of failing.
39. As a player on a very small terminal, I want a clear message telling me the size required, so that I know what to do.
40. As a player, I want the display to respond quickly enough that the Strobe tracks my Peg in what feels like real time.
41. As a player, I want the readout steady rather than twitching, so that I trust what it says.
42. As a player, I want to see the keymap on request, so that I do not have to remember it.

### When nothing useful is playing

43. As a player who has stopped playing, I want the app to say it is listening rather than report a note, so that it never invents a reading from silence.
44. As a player with an instrument plugged in but not being played, I want noise rejected, so that the app does not confidently report a nearly-in-tune note that I am not producing.
45. As a player who strums a chord by accident, I want the app to decline to report a Pitch, so that it does not name one arbitrary note of the chord as though I had played it alone.
46. As a player, I want the moment of pick attack ignored, so that the initial sharp transient does not throw the reading.
47. As a player, I want the last reading held and visibly dimmed once the note dies, so that I can still read what it said.
48. As a player on a noisy rig or with unusual gain settings, I want the Level gate to adapt to my noise floor, so that I do not have to calibrate anything by hand.

### Reference Pitch and persistence

49. As a player, I want to adjust the Reference Pitch, so that I can tune to an ensemble that is not at A440.
50. As a player, I want the Reference Pitch clamped to a sane range, so that I cannot accidentally set something absurd.
51. As a player, I want my Mode, Tuning and Reference Pitch remembered, so that the app opens in the state I left it.
52. As a player, I want a broken or hand-edited config file to be reported rather than to crash the app, so that a typo does not leave me unable to start.

## Implementation Decisions

### Architecture

Two threads and one lock-free queue, per the shape settled during design. The realtime `cpal` callback
does exactly one thing — deinterleave the chosen Input Channel into an `rtrb` SPSC queue — and never
allocates, locks, or analyses. The main thread polls keyboard input, drains a hop's worth of samples,
runs the pipeline, and renders. Analysis costs well under 1% of a core, and `ratatui` diffs its buffer,
so coupling analysis to the render loop is affordable. The failure mode if the terminal stalls is a
stale reading, which is benign for a tuner.

The window is sized in **milliseconds, not samples** (~171 ms), converted at whatever rate the device
reports. There is no resampler. This was exercised for real: the corpus was recorded at 44.1 kHz giving
a 7541-sample window, against 8208 at 48 kHz, with no special case anywhere.

### The audio-source seam

Audio input is an abstraction with two implementations — live `cpal` capture, and a WAV reader. This is
a testability decision, not a generality one: it makes every stage from `detect` through `session`
integration-testable against the committed corpus, deterministically, with no hardware and no terminal.
The prototype proved the seam works, and the corpus exists specifically to feed it.

### Modules

**Deep and pure — no I/O, no globals, the bulk of the value:**

- **`pitch`** — `Note`, `Pitch`, `Deviation`, `Reference Pitch`, and the twelve-tone equal temperament
  conversions between them. Tiny interface, used everywhere, essentially frozen once written.
- **`tuning`** — `Tuning`, `InstrumentString`, `Target Pitch`, derived `Capture Range`, and Guided Mode
  matching including `String Lock` and the fallback to naming a Note when nothing matches. A single
  matching entry point hides the neighbour-distance derivation, the absolute cap, and the awkward cases.
- **`detect`** — the two-stage detector of ADR 0001, lifting near-verbatim from the prototype. Very large
  functionality behind `analyse(window) -> Option<Reading>`.
- **`gate`** — Level and Clarity gating into three states, plus the noise-floor tracker of ADR 0005.
  Also lifts from the prototype.
- **`smoothing`** — median-3 followed by an EMA.
- **`strobe`** — the phase accumulator. Small, but ADR 0002 flags it as stateful and dependent on frame
  timing, which is exactly what should be isolated and tested rather than eyeballed.
- **`session`** — a reducer over readings and key events producing the state the UI renders. Owns Mode,
  Tuning, Reference Pitch, per-String status, and the Deviation Trail.

**Deliberately shallow adapters — all logic pushed out of them:**

- **`audio`** — device and channel enumeration, stream setup, the queue.
- **`config`** — TOML load and save, XDG path, missing-device fallback.
- **`ui`** — `ratatui` rendering, written as pure functions of session state.

### Detection

Follows ADR 0001 as amended by prototype measurement. Coarse McLeod estimate full-band for octave
robustness, then refinement against the autocorrelation peak at lag ≈ k·T, over an autocorrelation
low-passed at 8× the coarse f0 with a raised-cosine taper. Autocorrelation is computed by FFT
(`realfft`), which yields every lag in one pass — which is what the refinement stage needs.

Two implementation traps are recorded in ADR 0001 and must not be reintroduced, because both fail only
at the extremes of the range where casual testing never looks: the lag bounds need a few samples of
margin at each end, and the coarse ceiling and the refinement ceiling are different numbers.

**`k` is not capped.** Measurement showed refinement always uses the largest `k` the window allows, on
synthetic and real audio, through full decay; the peak-found test never rejected one.

### Capture Range derivation

Derived rather than configured, so no constant needs maintaining as Tunings are added. From a prototype
measurement that produced the ±250 cap:

```
range(String) = min( 250c , nearest_neighbour_gap / 2 − 15c )
```

The absolute cap exists because the outermost Strings have no neighbour on one side — without it the
lowest String claims any rumble beneath it, and a bass low E would read a 20 Hz thump as E1 and instruct
the player to tighten. Consequences worth knowing: DADGAD's G3 and A3 collapse to ±85¢, and the lowest
String of Open C and DADGAD hits the ±250 cap.

### Gating

Both gates are necessary and neither is redundant, established by measurement rather than argument:
silence below −60 dBFS reaches Clarity 0.94, so Clarity cannot reject silence; chords sit inside the note
Level range, so Level cannot reject chords.

```
Level below gate                    → Silent     hold last reading, dimmed
Level ok, Clarity < 0.90            → Unpitched  no reading at all
both ok                             → Pitched    live
```

**Clarity threshold is 0.90.** The originally assumed 0.8 sat at the exact median of real recorded noise
and would have admitted about a third of chord frames. Pick attack needs no separate suppression — the
transient is not yet periodic, so Clarity rejects it for free.

**The Level gate is adaptive**, per ADR 0005: a session-wide running minimum, gated a margin above it,
with an absolute ceiling. The ceiling is co-equal to the tracking rather than a safety net — if the app
starts while the player is already playing there is no quiet frame to learn from.

### Display

The Cockpit layout of ~120×36: half-block Headstock sprite, coarse gradient bar, animated Strobe, and a
braille Deviation Trail. Braille buys eight subpixels per cell by spending colour down to one, so it
takes thin traces; half-blocks keep two independently coloured subpixels, so they take the sprite. No
cell can have both — the split is forced, not chosen.

Chromatic Mode uses a simpler centred layout without the Headstock, consistent with it being both a
selectable Mode and the fallback when nothing matches a Capture Range.

Sign convention is fixed: negative Deviation is flat, meaning too slack, meaning tighten. The app never
names a rotation direction, only tighten or loosen — tightening always raises pitch, but which way the
Peg turns depends on the machine head and which side of the Headstock it sits on.

Palette is blue for flat, orange for sharp, bright neutral for in tune, per ADR 0003 — deliberately not
green and red. Hue therefore carries direction, so magnitude must come from bar position and Strobe rate.

Two generic Headstock sprites, one per instrument. The panel is a status display with instrument
character rather than wayfinding; the String number and Note label do the identification.

Small terminals degrade by dropping the Deviation Trail, then the Headstock, then the coarse bar, keeping
Note, Deviation and Strobe as the irreducible core, and refuse outright below roughly 40×12.

### Keymap

`Tab` toggles Mode · `t` cycles Tuning · `1`–`6` String Lock · `i` reopens the input picker ·
`+`/`-` adjust Reference Pitch · `?` keymap · `q`/`Esc` quit.

### Configuration

XDG config path. Persists Input Device, Input Channel, Tuning, Mode and Reference Pitch. A missing
remembered device falls back to the picker with an explanation, sharing one code path with a
mid-session disconnect. A malformed file is reported rather than fatal.

## Testing Decisions

A good test here asserts **external behaviour through a module's public interface** and says nothing
about how the module reaches its answer. Tests that reach into internals cannot survive the refactoring
they exist to enable. Concretely: assert the reported Deviation in cents, not the intermediate lag;
assert that a clip produces no Pitched frames, not that a particular gate branch was taken.

The corpus already establishes the prior art and the pattern, since the prototype validated the detector
exactly this way. Two complementary kinds of material, neither substitutable for the other:

- **Synthetic signals** have exactly known frequency, so they are the only thing that can assert
  accuracy. A real string has no exactly known pitch.
- **Recorded corpus clips** have real timbre, real inharmonicity and a real noise floor, so they are the
  only thing that can assert robustness.

### Modules under test

**`pitch`** — conversions both directions, Reference Pitch across its permitted range, and behaviour at
the B0 and E6 bounds.

**`tuning`** — Capture Ranges derived correctly for every shipped Tuning; no two Strings ever claiming the
same Pitch; DADGAD's G3/A3 pair narrowing as expected; outer Strings hitting the absolute cap; String
Lock overriding matching; the fallback to naming a Note outside all ranges.

**`detect`** — a synthetic sweep asserting worst-case error against exactly known truth, including a
weak-fundamental variant standing in for a bass pickup. Plus three regression tests pinning the bugs the
prototype found, each of which passed casual inspection: E6 must not read an octave low, B0 must be
detected at all, and refinement must not silently pin every `k` to 1.

**`gate`** — the noise-floor tracker must be a minimum and not an average, and specifically must not be
dragged upward by a sustained loud passage; the ceiling must keep the app from going deaf when it starts
mid-performance.

**`smoothing`** — an isolated outlier frame must not reach the output.

**`session`** — driven by readings and key events with no audio and no terminal: Mode toggling, String
Lock, in-tune status only within ±3 cents, Reference Pitch clamping, Trail accumulation, and holding the
last reading when Silent.

**End-to-end through the seam** — each corpus clip driven through the assembled pipeline: the noise clip
must produce no Pitched frames at all; the chord clip only a small minority; the bass clip must track the
E1/E2 alternation without octave errors; the guitar clip must hold its `k` through decay.

### Not under test

`ui` rendering. Snapshot tests would catch layout regressions, but they need rebaselining on every visual
tweak, and the visual design is the part most likely to keep moving. Revisit once it settles.

## Out of Scope

- **Reference tone playback and any audio output.** Excluded on a failure mode, not on effort — see
  ADR 0004. With a microphone input the app would hear its own tone, report zero cents, freeze the
  Strobe, and declare a String in tune that never contributed to the reading.
- **Polyphonic tuning** — reading all Strings from one strum. Needs multi-pitch detection and would
  change the detector fundamentally.
- **User-defined Tunings from configuration.** The Tuning table is data and Capture Ranges are derived,
  so this is a small later step, but validation and parse-error reporting are not v1 work.
- **Seven- and eight-String guitars, five- and six-String basses.** Chromatic Mode covers them.
- **Alternate temperaments and sweetened tunings.** Twelve-tone equal temperament only.
- **Intonation checking** — comparing a fretted note against the twelfth-fret harmonic.
- **Selectable Headstock styles.** One generic sprite per instrument.
- **A metronome**, or anything else not about pitch.
- **Verified Windows and Linux support.** `cpal` is cross-platform and nothing here is deliberately
  macOS-specific, but only macOS on arm64 is being tested.

## Further Notes

**The measured constants are defaults, not physical constants.** Clarity 0.90, the Level gate margin and
ceiling, and the low-pass multiple were calibrated against a single rig at one gain setting. They are
recorded with their evidence in `FINDINGS.md` on the `prototype/detector-probe` branch and should be
re-derived rather than trusted if behaviour looks wrong on other hardware.

**One accuracy limit is physical and not ours to fix.** Real strings are stiff, so their partials sit
progressively sharp and the best-fit period is pulled sharp with them. This appears as a systematic bias
of roughly one to a few cents depending on partial content, it affects every autocorrelation tuner, and
refinement narrows but cannot remove it. It does not endanger the requirement, because Measurement
Resolution is defined as what the app can *distinguish* rather than absolute correctness against ideal
twelve-tone equal temperament, and the bias points the same way on every String so relative tuning stays
true. The low-pass exists to reduce it.

**Still unmeasured:** the real inharmonicity coefficient of the strings in use, which sets the size of
that bias.

**The prototype is a primary source, not scaffolding to discard.** `prototype/detector-probe` holds the
harness, the corpus, and `FINDINGS.md`. Its `detect` and noise-floor modules were written to be lifted;
the CLI around them was not.
