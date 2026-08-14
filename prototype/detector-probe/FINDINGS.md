# detector-probe findings

Prototype answering: can the two-stage detector of ADR 0001 hit ≤1 cent, and what are the three
constants that argument could not settle?

Synthetic phase is **complete**. Real-audio phase is **not started** — it needs a guitar and bass
plugged in, and it is the only thing that can settle Clarity and Level.

## 1. The refinement mechanism works, and ADR 0001's premise is confirmed

On a strictly periodic harmonic stack across B0–E6 at 48 kHz, 171 ms window:

| | coarse | refined |
|---|---|---|
| worst error across sweep | +1.56¢ (at E6) | **0.01¢** |
| E4 (top guitar string) | −0.42¢ | −0.00¢ |
| E1 (bass low E) | −0.01¢ | −0.01¢ |

Coarse error grows with frequency and refinement flattens it — exactly the argument the ADR makes.
Single-stage genuinely does miss ≤1¢ at the top of the range (+1.56¢ at E6). A weak fundamental
(10%, mimicking a bass pickup) changes nothing: worst refined error 0.02¢.

## 2. `k` is NOT the binding constraint — the ADR's cap is the wrong lever

In **every** run, including extreme inharmonicity, `k_used == k_max_window`. The `found` criterion
never once rejected a high-`k` peak. Peaks at lag ≈ k·T stay findable; the limit is the window
(0.75·N) and our own probe limit, not the signal.

So the ADR's instruction to "verify the cap empirically against real recordings" is aimed at a
non-problem. There is no need for a cap around 30 for *findability* reasons.

## 3. Decay cannot bound `k`, and the ADR is half wrong about why

Runs at τ = 1.0 s, 0.3 s and 0.1 s produced **byte-identical** output, including clarity.

Autocorrelation peaks sit at exact multiples of the period regardless of amplitude envelope — decay
changes peak *heights*, not *locations* — and NSDF's `m(τ)` normalisation exists precisely to
compensate for the amplitude imbalance between the early and late halves of the window. Pure decay is
normalised away.

ADR 0001 said "a plucked string decays and its harmonics evolve, smearing autocorrelation peaks at
high multiples of the period." Only the *harmonics evolve* half can be true.

## 4. Inharmonicity is the real limit, and it is a bias, not noise

Real strings are stiff, so partial `h` sits at `h·f₀·√(1+Bh²)`. The signal is not strictly periodic,
and the best-fit period is pulled sharp by the upper partials.

At B = 5e-5, varying how many partials are present (which is what a low-pass filter controls):

| partials | coarse err | worst refined err |
|---|---|---|
| 8 | +1.10¢ | 1.16¢ |
| 16 | +4.00¢ | 3.71¢ |
| 32 | +9.70¢ | 4.24¢ |
| unlimited | +8.50¢ | 3.12¢ |

At 8 partials the error is **+1.10¢ at every note in the sweep** — uniform, signed, systematic.
Refinement reduces bias where `k` is large but cannot eliminate it, because there is no period to find.

Corroborating this: damping upper partials (differential harmonic decay, τ/h) took worst refined error
from 3.12¢ down to **0.61¢**. Removing high partials removes the bias.

### Consequence: resolution and absolute accuracy are different claims

`CONTEXT.md` defines **Measurement Resolution** as "the finest Deviation the app can *distinguish*" —
repeatability, not absolute correctness against ideal twelve-tone equal temperament. That requirement is
met with ~100× margin.

Absolute accuracy against an ideal harmonic model is bounded by the string's own physics, at roughly
1–4 cents depending on partial content. Every autocorrelation tuner has this. It matters less than it
looks, because the bias is in the same direction on every string, so *relative* tuning stays true.

### Open recommendation, not yet decided

Low-pass ahead of the refinement stage — the coarse pass only needs the right octave, so: coarse
detect, zero the FFT bins above ~8×f₀, recompute, refine. Nearly free, since the spectrum is already
in hand. **This is a design decision for the owner to make, not one the prototype gets to make.**

## 5. Two implementation gotchas worth not reintroducing

Both were real bugs in this prototype, found by the synthetic sweep, and both fail at the *edges* of
the range where casual testing never looks:

- **`min_lag` and `max_lag` need margin.** A peak at exactly `min_lag` is stepped over by a
  local-maximum scan starting at `min_lag+1`, and the octave below is locked instead — measured as
  **exactly −1200¢ at E6**. A peak at exactly `max_lag` falls outside a `t+1 < max_lag` guard —
  measured as **no detection at all at B0**. Three samples of margin at each end fixes both.
- **The coarse ceiling and the refinement ceiling are different numbers.** Coarse search needs lags up
  to `sr/30.87` ≈ 1555 samples. Refinement deliberately looks *far* past one period (k·T for E4 at
  k=30 is 4380). Clamping the probe to the coarse ceiling silently pins every `k` to 1 and makes
  refinement inert while still appearing to run.

## 6. Real-audio phase — constants pinned

Measured over four clips recorded at 44.1 kHz (`corpus-scratch-PROTOTYPE/`): a bass low E, a guitar
top E, a strummed chord, and an intended hum clip. 1112 note frames, 564 chord frames.

Note the sample rate: the device ran at **44.1 kHz, not 48 kHz**, giving a 7541-sample window rather
than 8208. Sizing the window in milliseconds rather than samples handled this with no special case.

### Clarity threshold — 0.90, not the provisional 0.8

| threshold | note frames kept | chord frames admitted |
|---|---|---|
| 0.80 | 95.5% | **31.6%** |
| 0.85 | 92.7% | 9.2% |
| **0.90** | **91.3%** | **4.1%** |
| 0.95 | 76.6% | 2.3% |

0.8 lets nearly a third of chord frames through. 0.90 is the knee. The residual 4% arrives as isolated
frames, which median-3 filtering suppresses before they reach the display.

### Level floor — −55 dBFS, with a caveat

The guitar clip's level histogram has a genuinely empty valley:

```
 -70..-65 dB  ##############   56     <- silent gaps
 -65..-60 dB  ####             17
 -60..-55 dB                    0     <- nothing here
 -55..-50 dB                    0
 -50..-45 dB  ##                9     <- real notes
 -45..-40 dB  #############   155
 -40..-35 dB  ###############  220
 -35..-30 dB  ########         107
```

**Caveat: this is gain-dependent.** An interface's input gain moves the whole distribution, so a fixed
dBFS floor calibrated here will not transfer to a different rig. Whether to use a fixed floor or track
the noise floor adaptively is a design decision, not something the prototype settles.

### Both gates are necessary — neither is redundant

- Frames below −60 dBFS (actual silence) reach clarity **0.94**. Clarity alone cannot reject silence.
- Chord frames sit at −54..−33 dBFS, inside the note range. Level alone cannot reject chords.

### Q1 answered definitively on real strings

`k_max_signal == k_max_window` in every level band: bass p50 = 5 against a window limit of 5, guitar
p50 = 42 against 42. And it does not decay — the guitar holds k = 42 in both the −30..−40 dB band and
below −40 dB. `k` is never limited by the signal, on real audio, through full decay. Confirms §2 and §3.

### No octave errors on bass low E

The bass clip alternates E1 and E2 roughly every second (the player's doing, not the detector's). That
makes it unintentionally good material, because the E1↔E2 boundary is exactly where an octave-error-prone
detector fails. All ten transitions came through as clean runs of 40–74 frames with no flapping:

```
_39 ^74 _45 ^52 _61 ^57 _55 ^42 _69 ^13     ( _ = E1, ^ = E2, one char per frame )
```

MPM's "first peak within 90% of the tallest" rule is doing its job. Picking the tallest peak outright
is what produces octave errors, and this is the evidence that the distinction matters in practice.

## 7. Low-pass before refinement — measured, adopted

Refinement now runs on a band-limited autocorrelation, low-passed at 8x the coarse f0 with a
raised-cosine taper (a brick wall rings and plants sidelobes exactly where we then look for peaks).
Costs one extra inverse FFT and no extra forward transform, because the ACF of a filtered signal is
`IFFT(|H|^2 . |X|^2)` and the power spectrum is already stored.

At B = 5e-5, refined error:

| note | no low-pass | low-pass 8x |
|---|---|---|
| B0 | +3.12c | -0.69c |
| E1 | +2.04c | +0.07c |
| E2 | +2.21c | +0.63c |
| G3 | +1.37c | +0.76c |
| A4 | +0.40c | +0.72c (worse) |
| **worst** | **3.12c** | **0.95c** |

Worst case crosses under the 1c requirement. Low notes gain most, having the most partials; a couple
of high notes lose slightly. Strictly periodic signals are unchanged at 0.01c, so it costs nothing
where there is no inharmonicity to correct.

Caveat: at B = 2e-4 the benefit largely vanishes. But a uniform B across the range is unphysical --
thick wound bass strings have low B, thin plain steel has high B -- so that case does not correspond
to a real instrument.

## 8. Adaptive Level gate — measured, adopted

A fixed dBFS floor cannot work: the floor measured -78 dBFS on this rig at this gain, and the gain
knob slides the whole distribution. Two simpler designs were tried and rejected **on measurement**:

- **Averaging / EMA** — dragged upward by loud passages until it gates off the notes it exists to pass.
- **Sliding-window minimum** — same flaw over the window length. A 10 s window during continuous
  playing tracked the *playing* level (-27.9 dBFS), not the noise floor. Three of four clips ended up
  gated by the safety ceiling rather than by any tracking.

Adopted: **session-wide running minimum, +18 dB margin, ceiling -50 dBFS, upward leak 0.1 dB/s.** A
noise floor is a property of the rig and gain, so it barely moves within a session; a minimum captures
it at the first quiet moment and is immune to loud passages by construction. The leak handles a
genuinely noisier environment -- at 1 dB/s it drifted 12 dB across a 12 s clip, far faster than any real
floor moves, hence 0.1.

The ceiling is **co-equal to the tracking, not a safety net**: if the app starts while the player is
already playing, there is no quiet frame to learn from, and the ceiling is the only thing preventing
deafness until one arrives.

| clip | tracked floor | gate | passes both gates |
|---|---|---|---|
| hum | -78.9 dBFS | -60.9 (adaptive) | **0.0%** |
| bass low E | -49.1 | -50.0 (ceiling) | 89.9% |
| guitar top E | -67.3 | -50.0 (ceiling) | 86.3% |
| chord | -75.3 | -57.3 (adaptive) | 4.1% |

Loosening the ceiling from -40 to -50 recovered guitar frames (80.3% -> 86.3%) but raised chord
admission from 2.7% to 4.1% -- i.e. the Level gate stops helping against chords and Clarity carries it
alone. Accepted, since those frames arrive isolated and median-3 removes them.

## 9. Still open

- **The hum clip is unusable.** `hum.wav` is digital silence: 99.85% of samples are exactly zero, peak
  1 LSB, −118 dBFS RMS. The input was dead — nothing connected, or the wrong channel. It establishes a
  true-silence baseline but says nothing about mains hum, which is the realistic rejection case and the
  one that fires when the player is *not* playing. **Needs re-recording with a live input.**
- **The Level floor is provisional** until that re-record, and gain-dependence is unresolved.
- **Actual `B` for these strings** is unmeasured, and it sets the accuracy bias in §4.

## Reproducing

```sh
cargo run -- --synthetic-sweep                                  # accuracy, exact truth
cargo run -- --synthetic-sweep --weak-fundamental               # bass pickup
cargo run -- --synthetic-sweep --decay 0.1                      # decay (no effect — see §3)
cargo run -- --synthetic-sweep --inharmonicity 5e-5 --partials 8
cargo run -- --synthetic 195.998 --inharmonicity 5e-5 --k-table # per-k detail

cargo run -- --list
cargo run -- --device Scarlett --channel 1 --tag note --secs 20
cargo run -- --tag bass-low-e --record bass-low-e --secs 8
```
