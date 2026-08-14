# The Level gate tracks the noise floor instead of using a fixed dBFS threshold

A constant is the obvious implementation and it cannot work. The noise floor measured −78 dBFS on one
interface at one gain setting; turning the gain knob slides the entire distribution, so any dBFS number
calibrated on one rig is wrong on the next. Getting it wrong in one direction shows noise as if it were
a note, and in the other leaves the app deaf.

Two simpler adaptive designs were tried and rejected on measurement, not on reasoning:

- **Averaging / EMA of recent levels** — dragged upward by loud passages, climbing until it gates off
  the very notes it exists to pass.
- **Sliding-window minimum** — the same flaw over the window's length. A 10 second window during
  continuous playing tracked the *playing* level at −27.9 dBFS rather than the noise floor, and three of
  four test clips ended up gated by the safety ceiling rather than by any tracking at all.

We therefore track a **session-wide running minimum**, gate 18 dB above it, cap the gate at −50 dBFS,
and let the floor leak upward at 0.1 dB/s. A noise floor is a property of the rig and gain rather than of
the moment, so it barely moves within a session: a minimum captures it at the first quiet instant and is
immune to loud passages by construction. The leak covers a genuinely noisier environment — at 1 dB/s it
drifted 12 dB across a 12 second clip, far faster than any real floor moves.

## Consequences

**The ceiling is co-equal to the tracking, not a safety net.** If the app starts while the player is
already playing there is no quiet frame to learn from, and the ceiling is the only thing preventing
deafness until one arrives. Anyone tempted to remove it as belt-and-braces should note that it was the
*only* active mechanism in three of the four measured clips.

Choosing the ceiling trades the two gates off against each other. Loosening it from −40 to −50 dBFS
recovered genuine guitar frames (80.3% → 86.3% passing) but raised chord admission from 2.7% to 4.1% —
that is, the Level gate stops contributing to chord rejection and Clarity carries it alone. Accepted,
because those frames arrive isolated and the median-3 filter removes them before display.

These numbers are calibrated against a single rig. They are defaults, not physical constants.
