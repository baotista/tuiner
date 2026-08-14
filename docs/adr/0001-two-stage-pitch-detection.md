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

Measured on real audio (bass low E, guitar top E, strummed chord; 1112 note frames, 564 chord frames,
recorded at 44.1 kHz — which incidentally exercised the millisecond-sized window with no special case):

- **Clarity threshold is 0.90, not the provisional 0.8.** At 0.8 nearly a third of chord frames pass; at
  0.90, 4.1% pass while 91.3% of note frames are kept, and those stragglers arrive isolated so median-3
  filtering removes them before display.
- **`k` is never signal-limited on real strings either.** `k_max_signal == k_max_window` in every level
  band, and it does not decay — the guitar held k = 42 both above and below −40 dBFS.
- **No octave errors on bass low E**, the classic MPM failure mode. The clip happens to alternate E1
  and E2 roughly every second; the detector tracked all ten transitions as clean runs of 40–74 frames
  with no flapping at the boundary.
- **Both gates are load-bearing.** Silence below −60 dBFS reaches clarity 0.94, so Clarity cannot reject
  silence; chords sit at −54..−33 dBFS inside the note range, so Level cannot reject chords.
- **The Level floor is gain-dependent** and cannot be a constant — see ADR 0005.

A re-recorded noise clip (guitar plugged in, not played — the true "not playing" state) settled the last
of it: that noise reads as **clarity 0.75–0.83, median exactly 0.80**, so the original provisional
threshold sat in the middle of the noise distribution and would have admitted about half of those
frames. At 0.90 it is rejected entirely. Note also what the noise is *reported as* — 51.82 Hz, G#1 −3¢,
an entirely plausible-looking nearly-in-tune note. That is the whole case for gating: the detector never
declines to name a pitch, it just names a wrong one convincingly.

Still unmeasured: the real value of `B` for these strings.

## Low-pass before refinement — adopted

The coarse pass stays full-band, since it only needs the right octave. Refinement runs on an
autocorrelation low-passed at 8×f₀ with a raised-cosine taper — a brick wall rings and plants sidelobes
exactly where stage 2 then looks for peaks. It costs one extra inverse FFT and no extra forward one,
because the ACF of a filtered signal is `IFFT(|H|²·|X|²)` and the power spectrum is already stored.

Measured at B = 5e-5, worst refined error across the sweep falls from **3.12¢ to 0.95¢**, crossing under
the requirement. Low notes gain most, having the most partials (B0: +3.12¢ → −0.69¢); a couple of high
notes lose slightly (A4: +0.40¢ → +0.72¢). Strictly periodic signals are unchanged at 0.01¢, so it costs
nothing where there is no inharmonicity to correct.

The benefit largely vanishes at B = 2e-4, but a uniform B across the whole range is unphysical: thick
wound bass strings have low B and thin plain steel has high B, so that case is not a real instrument.

### Correction, from building the real crate: the low-pass needs a floor

"Strictly periodic signals are unchanged at 0.01¢" above is wrong at the bottom of the range. It held
for the sweep as tested here, where only B0 sits below five periods per window — but a semitone-by-semitone
probe from B0 to E2 (still strictly periodic, no inharmonicity) found the raised-cosine taper itself
distorting the peak whenever the window holds only 3-4 periods (`k_max_window`), up to **1.29¢ at C#1
and D1** and **1.69¢ at B0** — regressing the very ≤1¢ requirement the low-pass exists to protect,
with nothing for it to correct at those points. From 5 periods on (E1 and above) the taper is safe and
the stated benefit holds.

Adopted fix: skip the low-pass when `k_max_window < 5`, falling back to the plain full-band NSDF for
refinement at those few, very low notes. This gives up the inharmonicity correction exactly where ADR
0001's own sweep (§ low-pass table) shows it would help real strings most — B0's inharmonic case improved
most of any note, +3.12¢ → −0.69¢ — but that benefit was only ever measured with inharmonicity present,
never in combination with this taper artifact, so there is no real-string evidence it survives the
combination. Revisit if a narrower or gentler taper is found that keeps the benefit at low `k` without
reintroducing the artifact.
