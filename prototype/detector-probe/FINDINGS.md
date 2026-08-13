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

## 6. Still unanswered — needs real audio

| Constant | Status |
|---|---|
| Clarity threshold | provisional 0.8. Clean synthetic 0.99–1.00; MacBook room noise at −60 dBFS gave 0.11–0.68 (p95 0.64). 0.8 sits in the gap, but real mains hum and a strummed chord are untested. |
| Level floor | unmeasured. MacBook mic room noise was −62 to −50 dBFS; a plugged-in guitar via an interface will be far louder. Needs both. |
| Actual B for real strings | unmeasured, and it sets the accuracy bias in §4. Wound bass strings and plain steel differ. |

Notably, **0 of 135 room-noise frames returned "no periodicity found"** — the detector always finds
*something*. Clarity does all of the rejection work, exactly as the design assumed.

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
