# Hand-rolled two-stage pitch detection

We require ≤1 cent Measurement Resolution across 30.87 Hz (B0) to 1319 Hz (E6). A conventional
single-pass YIN/McLeod detector cannot deliver this at the top of that range: it reports the period as
a lag in samples, cent error scales as `1731 × δ_lag / lag`, and even with parabolic interpolation
(δ ≈ 0.2 samples) the error is ±2.4 cents at E4 — the top string of a standard guitar — and ±9.5 cents
at E6. That is as wide as the entire ±3 cent In-Tune Tolerance, so the readout would visibly twitch.

We therefore implement detection ourselves in two stages: McLeod (MPM) for an octave-robust coarse
period, then refinement against the autocorrelation peak at lag ≈ k·T for the largest k that fits the
window, which divides relative error by k. Counter-intuitively this makes precision *improve* with
frequency, exactly compensating the weakness of the single-stage approach.

## Considered options

- **`pitch-detection` crate** — last released June 2022, and single-stage regardless, so it would not
  have met the resolution requirement even if maintained.
- **`aubio-rs`** — pulls in a C library and system build dependency for one function.
- **MPM + FFT phase-vocoder refinement** — comparable or better precision and flat across frequency,
  but requires phase-unwrapping logic that fails silently when it picks the wrong branch, and depends
  on a strong fundamental bin — which is exactly what a bass pickup does not give us. Rejected on
  fragility, not on cost.
- **Single-stage MPM, loosen the requirement to ~3 cents** — rejected because measurement noise would
  then equal the in-tune band on the top strings.

## Implementation note

The autocorrelation both stages need is computed by FFT (`realfft` over `rustfft`, zero-padded to
16384), not by direct dot product. At the ~48 frames/sec response rate we chose, brute force would cost
roughly 600M MAC/s — a fifth of a core, burned continuously while the app idles. The FFT is ~16× cheaper
and, more usefully, yields *every* lag in one pass, so stage 2 finds its peak at lag ≈ k·T without a
second search. NSDF normalisation still needs the `m(τ)` running-sum term, obtained via prefix sums.

## Validated by prototype

Measured on branch `prototype/detector-probe` (see its `FINDINGS.md`). The premise holds: across
B0–E6 on a strictly periodic harmonic stack, worst coarse error is +1.56¢ at E6 — genuinely missing the
requirement, as argued — and worst refined error is **0.01¢**. A weak fundamental mimicking a bass
pickup changes nothing (0.02¢).

Two claims in the original reasoning were wrong, and are corrected here rather than left to mislead:

- **`k` is not the lever, and does not need a cap for findability.** In every run, including extreme
  inharmonicity, refinement used the largest `k` the window allowed and the "peak found" test never
  once rejected one. High-`k` peaks stay findable. `k` is bounded by window length, not by the signal.
- **Decay cannot degrade refinement.** Runs at τ = 1.0, 0.3 and 0.1 s were byte-identical. Peaks sit at
  exact multiples of the period whatever the amplitude envelope, and NSDF's `m(τ)` normalisation exists
  precisely to compensate the early/late amplitude imbalance. Only the "harmonics evolve" half of the
  original worry was real.

What actually limits accuracy is **inharmonicity**, and it behaves as a *bias* rather than as noise:
real strings are stiff, partial `h` sits at `h·f₀·√(1+Bh²)`, and the upper partials pull the best-fit
period sharp. At B = 5e-5 the error is uniform across the whole range — +1.10¢ at every note with 8
partials present, rising to +9.70¢ coarse with 32. Refinement narrows it but cannot remove it, because
an inharmonic string has no period equal to `1/f₀` to find.

This does not endanger the requirement, because **Measurement Resolution is defined as what the app can
distinguish, not absolute correctness against ideal twelve-tone equal temperament** — repeatability is
met with roughly 100× margin. The residual bias is a property of the string, is shared by every
autocorrelation tuner, and points the same way on every string, so relative tuning stays true.

## Consequences

Two edge-case implementation traps, both found by the synthetic sweep and both invisible to casual
testing because they only fire at the extremes of the range:

- `min_lag` and `max_lag` need a few samples of margin. A peak at exactly `min_lag` is stepped over by
  a local-maximum scan starting at `min_lag+1`, locking the octave below — measured as exactly −1200¢
  at E6. A peak at exactly `max_lag` falls outside a `t+1 < max_lag` guard — measured as no detection
  at all at B0.
- The coarse ceiling and the refinement ceiling are **different numbers**. Coarse search needs lags to
  `sr/30.87` ≈ 1555 samples; refinement deliberately looks far past one period (`k·T` for E4 at k=30 is
  4380). Clamping refinement to the coarse ceiling pins every `k` to 1 and makes the whole second stage
  inert while still appearing to run.

Still outstanding, and answerable only with real audio: the Clarity threshold, the Level floor, and the
real value of `B` for the strings in question. Note that not one of 135 room-noise frames returned "no
periodicity found" — the detector always finds *something*, so Clarity carries the entire burden of
rejection.

An open recommendation this raised, deliberately left undecided: low-pass ahead of refinement. The
coarse pass only needs the right octave, so zeroing FFT bins above ~8×f₀ and recomputing would cut the
inharmonicity bias substantially — corroborated by damping upper partials taking worst refined error
from 3.12¢ to 0.61¢ — at almost no cost, since the spectrum is already in hand.
