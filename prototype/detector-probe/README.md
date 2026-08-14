# detector-probe — PROTOTYPE, THROWAWAY

Not production code. No tests, no error handling beyond what makes it run. It exists to answer one
question and is kept only as a primary source for the decision it settled.

**Read [FINDINGS.md](FINDINGS.md) for the answers.** This file is just how to run it.

## The question

Can the two-stage detector of `docs/adr/0001-two-stage-pitch-detection.md` hit ≤1 cent across
B0–E6, and what are the three constants that argument cannot settle — the `k` cap, the Clarity
threshold, and the Level floor?

## What is worth keeping

`src/detect.rs` is written as a pure, liftable module: no audio I/O, no printing, no global state.
Feed it a window of mono samples, get a `Reading` back. That is the part that folds into the real
crate. `src/main.rs` is throwaway scaffolding around it.

## Running

```sh
cd prototype/detector-probe
cargo run -- --synthetic-sweep     # no hardware needed — start here
cargo run -- --list                # then plug something in
```

### Synthetic — exact known truth, so accuracy is assertable

| Flag | Effect |
|---|---|
| `--synthetic-sweep` | 14 notes, B0 to E6, error in cents for coarse vs refined |
| `--synthetic <hz>` | one frequency instead of the sweep |
| `--weak-fundamental` | fundamental at 10% — mimics a bass pickup |
| `--decay <s>` | exponential amplitude decay |
| `--harmonic-decay <s>` | partial `h` decays as `τ/h` — real strings damp highs faster |
| `--inharmonicity <B>` | stiff-string: partial `h` at `h·f₀·√(1+Bh²)`. Try `5e-5` |
| `--partials <n>` | cap partial count — equivalent to a low-pass at `n×f₀` |
| `--k-table` | per-`k` detail: NSDF height, implied Hz, whether the peak was found |
| `--lowpass <n>` | low-pass before refinement at `n`x the coarse f0. Adopted at `8` |

### Real audio — real timbre, so robustness is assertable

| Flag | Effect |
|---|---|
| `--device <substring>` | case-insensitive match against device name |
| `--channel <n>` | 1-based, matching an interface's front-panel labels |
| `--tag <label>` | labels the run; one run per condition keeps segments clean |
| `--secs <n>` | run length, default 20 |
| `--record <name>` | writes `corpus-scratch-PROTOTYPE/<name>.wav` (mono 16-bit) |
| `--wav-in <path>` | analyse a recorded clip instead of live audio |
| `--csv` | per-frame values to stdout for offline analysis |

Run one session per condition rather than tagging interactively — clean segments, no mis-tagging:

```sh
cargo run -- --device Scarlett --channel 1 --tag note    --secs 20   # single plucked notes
cargo run -- --device Scarlett --channel 1 --tag hum     --secs 10   # nothing plugged in
cargo run -- --device Scarlett --channel 1 --tag chord   --secs 15   # strummed — must be rejected
cargo run -- --device Scarlett --channel 1 --tag silence --secs 10   # muted strings
```

Each run ends with percentile summaries of Level, Clarity and `k_max`, plus a breakdown of `k_max`
by level bucket — that last table is how you see whether `k` degrades as a note decays.

`--record` also makes this the tool that captures the test corpus. `corpus-scratch-PROTOTYPE/` is
scratch — wipe it freely; it is gitignored. Promote a clip into [`corpus/`](corpus/README.md) once it
proves worth keeping; those are tracked, and re-running against them is how the constants stay
reproducible instead of resting on someone's memory of a session.
