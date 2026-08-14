# Recorded corpus

The real-timbre half of the test corpus. Synthetic signals validate *accuracy* because their
frequency is exactly known; these validate *robustness*, which synthetic signals cannot — real
inharmonicity, real decay, real noise floor.

All mono 16-bit at 44.1 kHz, captured through the same path the app uses.

| clip | contents | what it pins |
|---|---|---|
| `hum.wav` | guitar plugged in, not played — the true "not playing" state | Clarity ceiling of noise (0.83) and the noise floor (−78.9 dBFS) |
| `bass-low-e.wav` | bass, alternating E1 and E2 roughly every second | Octave stability across the E1↔E2 boundary, where octave errors happen |
| `gtr-top-e.wav` | guitar top E, single plucks into full decay | That `k` does not degrade through decay |
| `chord.wav` | strummed chord, repeatedly | Clarity floor that must be rejected (4.1% admitted at 0.90) |

`corpus-scratch-PROTOTYPE/` is where `--record` writes. It stays gitignored — promote a clip to
this directory when it proves worth keeping.

```sh
cargo run -- --wav-in corpus/hum.wav --lowpass 8
```
