# Tuiner

A terminal instrument tuner for guitar and bass. It listens to a chosen audio input, works out
what pitch is sounding, and shows the player which way to turn the peg.

## Language

### Pitch and deviation

**Pitch**:
The fundamental frequency currently sounding, in hertz. The single value the app extracts from audio.
_Avoid_: note (a Pitch is a measurement; a Note is a name), frequency, tone

**Note**:
A named position in twelve-tone equal temperament, such as `E2` or `F#3`. A label, not a measurement.
_Avoid_: pitch, tone, key

**Reference Pitch**:
The frequency assigned to `A4`, from which every other Note's frequency is derived.
_Avoid_: concert pitch, calibration, A440

**Deviation**:
How far the sounding Pitch sits from a Target Pitch, in cents. Signed: negative is flat, positive is sharp.
_Avoid_: offset, error, delta, difference, cents (cents is the unit, not the quantity)

**In-Tune Tolerance**:
The Deviation band inside which a String counts as correctly tuned — currently ±3 cents.
_Avoid_: threshold, accuracy, precision, margin

**Measurement Resolution**:
The finest Deviation the app can distinguish — required to be ≤1 cent, so the display moves smoothly
rather than visibly stepping. Distinct from In-Tune Tolerance, which is a verdict, not a capability.
_Avoid_: accuracy, precision

### Instrument and tuning

**Tuning**:
A named set of Target Pitches, one per String, such as `Open D` or `DADGAD`. A flat concept — the app
draws no distinction between open, modal, and standard tunings.
_Avoid_: preset, temperament, key, alternate tuning

**String**:
One of the instrument's playable strings, identified by its position — numbered from 1 at the
highest-pitched string upwards, following guitarists' convention. Under a given Tuning each String has
exactly one Target Pitch. Named `InstrumentString` in code, to avoid colliding with Rust's `String`.
_Avoid_: course, channel

**Peg**:
The tuning machine that tensions one String. One Peg per String, but unlike a String a Peg has a
physical position on the Headstock — which is the whole reason the concept exists, since telling a
player *which* peg to turn is the point.
_Avoid_: machine head, tuner (reserved for the app itself), knob

**Headstock**:
The arrangement of an instrument's Pegs in their real physical positions. Not derivable from String
numbering: a six-in-line headstock and a three-a-side headstock hold the same six Strings in entirely
different places.
_Avoid_: neck, head, peghead

**Target Pitch**:
The frequency a String is supposed to sound, determined by its Note under the current Tuning and the
Reference Pitch.
_Avoid_: desired pitch, goal, expected frequency

### Audio input

**Input Device**:
An audio capture device the operating system exposes, such as a built-in microphone or a USB audio
interface. Offers one or more Input Channels.
_Avoid_: input, source, soundcard, mic, interface

**Input Channel**:
A single mono channel within an Input Device. The app listens to exactly one Input Channel at a time —
channels are never mixed together.
_Avoid_: track, port, jack, line

**Level**:
The short-term loudness of an Input Channel, shown while choosing an input so the player can see which
channel their instrument is actually plugged into.
_Avoid_: volume, gain, amplitude, signal strength

**Clarity**:
How strongly periodic the incoming audio is, from 0 to 1. High for a single plucked string, low for
mains hum, speech, a strummed chord, or the chaotic instant of a pick attack. A Pitch is only reported
when Clarity is high enough to trust it.
_Avoid_: confidence, quality, SNR, certainty, periodicity

### Modes

**Guided Mode**:
The app knows the Tuning, matches the sounding Pitch to the nearest String in that Tuning, and reports
Deviation against that String's Target Pitch. Strings may be played in any order.
_Avoid_: targeted mode, instrument mode, preset mode

**Chromatic Mode**:
The app has no Tuning. It reports Deviation against whichever Note in twelve-tone equal temperament is
nearest the sounding Pitch. Both a mode the player can choose and the fallback Guided Mode drops into
when a sounding Pitch falls outside every String's Capture Range — there being nothing to guide toward,
it stops guiding and simply names the Note.
_Avoid_: free mode, manual mode

**Capture Range**:
The Deviation band around a String's Target Pitch within which a sounding Pitch is attributed to that
String. Derived per String from half the distance to its nearest neighbouring Target Pitch, so two
Strings can never both claim the same Pitch, and additionally capped in absolute terms — the outermost
Strings have no neighbour on one side, and without a cap the lowest String would claim any rumble
beneath it.
_Avoid_: tolerance, window, snap range, threshold

**Strobe**:
A display whose apparent motion represents Deviation: it drifts one way when flat, the other when
sharp, slows as the Pitch approaches its Target Pitch, and freezes when the two coincide. Unlike a
scale, it has no end stops — a Deviation too small to plot is still visible as motion too slow to
finish.
_Avoid_: animation, spinner, wheel

**Deviation Trail**:
The recent history of Deviation drawn against time, showing whether a String is converging, overshooting,
or drifting back out after being turned.
_Avoid_: graph, history, plot, chart

**String Lock**:
A state in which the player has named a String explicitly and the app stops matching by Capture Range,
reporting Deviation against that String however far away the sounding Pitch is. The way to fit a fresh
string, which starts far too flat to be matched.
_Avoid_: pinning, selection, manual mode
