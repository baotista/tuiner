# Input only: no reference tone, no audio output

Nearly every tuner can sound a reference tone, so its absence here will look like an omission. It is a
boundary we drew deliberately, and the reason is a failure mode rather than effort.

If the chosen Input Channel belongs to a built-in microphone, a reference tone played through the
speakers is picked up by that microphone. The detector then locks onto a mathematically perfect sine at
exactly the Target Pitch, reports a Deviation of 0.0 cents, and freezes the Strobe — declaring the String
in tune while the actual string is tens of cents out and contributed nothing to the reading. The app
would be confidently lying, and the player has no way to tell.

Mitigations all disappoint. Warning the user depends on guessing whether a device is a microphone from
its name, which is unreliable, and a wrong guess produces exactly the silent lie above. Muting detection
whenever the tone sounds is structurally safe but removes the point: you can no longer watch the Strobe
while hearing the reference, which is how tuning by ear actually works.

We therefore ship one `cpal` input stream and no output stream at all. The app cannot deceive itself,
by construction rather than by care.

## Consequences

Adding a reference tone later is additive — a second stream and a synthesiser — but whoever adds it must
solve the microphone feedback case first, not merely notice it.
