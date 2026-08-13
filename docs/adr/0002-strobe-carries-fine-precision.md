# The Strobe, not a needle, carries fine precision

Every conventional tuner shows Deviation on a bounded scale, and in a terminal that scale runs out of
room exactly where it matters: our ±3 cent In-Tune Tolerance is 6% of a ±50 cent bar, so on a 40-column
meter the entire endgame of tuning happens inside two characters. Zooming the scale as the Pitch
approaches works, but the marker then jumps sideways while the player has changed nothing, which reads
as going backwards.

We therefore make a Strobe the primary fine indicator and leave the bar coarse. Because a Strobe encodes
Deviation as *rate of apparent motion* rather than position, its precision has no floor: −20 cents races
past at 2.3 cycles/sec, −3 cents crawls at 0.34, and −0.5 cents takes eighteen seconds to complete one
cycle — plainly distinguishable from frozen, and unplottable on any scale we could fit in a terminal.

## Consequences

The Strobe needs a phase accumulator advanced by `2π·(f_detected − f_target)·dt` per frame, which means
the display is stateful and depends on frame timing rather than being a pure function of the latest
reading. Frame delivery must therefore be reasonably regular, and a long stall will show as a phase
jump. It also means the Strobe cannot render meaningfully without a Target Pitch, so in Chromatic Mode
it strobes against the nearest Note instead.

Braille and half-block cells split by role, and the split is forced rather than chosen: braille buys 8
subpixels per cell by spending colour down to one per cell, so it takes the Deviation Trail and other
thin traces; half-blocks keep two independently coloured subpixels, so they take the Headstock sprite.
No cell can have both.
