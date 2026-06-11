# VAD & Voice-Detection: Research Findings and Fixes

Investigation into the JFC voice system (`crates/jfc-voice`) prompted by three
reported symptoms:

1. **"After a long quiet period it stops detecting when I start talking again."**
2. **"It thinks I'm done speaking when I take a breath / pause between stanzas."**
3. **"Movies/voices playing in the background get detected as my voice."**

This document records the root-cause analysis, the literature that grounds it,
the fixes implemented, and the deliberately-out-of-scope follow-ups. Citations
are to primary sources (papers downloaded + PDF→text under `/tmp/vadpapers`, the
Silero model source, and the `voice_activity_detector` crate), not to a search
engine's summary.

---

## 0. What actually runs

There are **two** engines behind one `VadEvent` contract (`vad.rs`):

| Engine | File | How it decides "speech" |
| --- | --- | --- |
| **Energy VAD** (default-built) | `vad.rs` | Hand-tuned acoustic features: adaptive noise floor + hysteresis + periodicity/modulation/flatness gates + hangover. |
| **Neural VAD** (Silero v5 ONNX) | `neural_vad.rs` | Small recurrent net → per-frame speech *probability*; same onset/hangover state machine on top. |

The `vad-neural` feature **is in jfc's default feature set**
(`crates/jfc/Cargo.toml:116`), and `VadDetector::build_default()` selects
**Neural** when the feature is compiled and the model loads. So in a normal jfc
build, **the neural engine is what you're using** — which is why symptom #1
("the neural system") pointed there.

---

## 1. Symptom #1 — long-idle non-detection = Silero recurrent-state drift

### Root cause (confirmed in source, not inferred)

Silero VAD v5 is an **LSTM-based recurrent network**. The bundled
`voice_activity_detector` crate (v0.2.1, `src/vad.rs`) carries the recurrent
state in a `state: ndarray::ArrayD<f32>` field of shape `2×1×128`. Each
`predict()` call:

- feeds the current state in as the model's `state` input,
- reads the model's `stateN` output, and
- **writes it back** into `self.state` for the next call.

The state is only ever zeroed by an explicit `reset()`. There is no automatic
decay.

In our recorder (`recorder.rs`), `vad_listen_loop` creates **one** detector
(`recorder.rs:460`) and then sits in an idle "wait for speech" loop
(`recorder.rs:475-500`) feeding **every** captured frame — including minutes of
silence — into `detector.push()` → `predict()`. `detector.reset()` is only
called **after** an utterance completes (`recorder.rs:635`). **During a long
idle wait the LSTM state is never reset.**

This is the textbook **RNN hidden-state drift / saturation** failure on long
sequences:

- Brenndörfer, *Vanishing Gradients: Why RNNs Fail on Long Sequences* (2025):
  "hidden states drift away from zero as the network processes sequences… and
  the effective gain drops… Saturating [activations]…" — i.e. a long
  monotonous input pushes the recurrent state into a regime where it stops
  responding crisply to new input.
- A 2025 PMC convergence/stability analysis of LSTMs explicitly limits hidden
  state dimension and warns about "state drift and gradient accumulation on long
  sequences."

The practical consequence: after a long quiet stretch the carried state biases
the probability for the chunks that follow, so the **onset of your next
utterance is missed or badly delayed** — exactly the reported symptom.

### What production stacks do

- **Silero maintainers** (GitHub Discussions #572, and the project README's
  streaming examples) reset the model state each time a stream "changes or
  ends," and the streaming utilities expose `reset_states()`.
- **Pipecat's** `SileroVADAnalyzer` force-resets the model on a fixed
  wall-clock cadence — `_MODEL_RESET_STATES_TIME = 5.0` seconds — **regardless of
  activity**, precisely to stop long-run drift.

Note (per advisor): Pipecat's reset is a *blunt periodic* reset. Resetting
*during* speech can itself glitch detection mid-word, so the reset must be gated
to non-speech.

### Fix (`neural_vad.rs`)

While **idle** (not `in_speech`), count consecutive non-speech chunks and reset
the Silero state after a sustained run:

- `JFC_VAD_NEURAL_IDLE_RESET_MS` (default **5000 ms**, matching Pipecat; `0`
  disables).
- The reset is **gated to the idle state** — it can never fire mid-utterance, so
  it cannot wipe in-word context.
- `idle_silent_chunks` resets to 0 on any voiced chunk and on entering speech.

Because the recorder's idle wait-loop calls `detector.push()`, this fix applies
automatically with no recorder change.

Regression tests: `idle_state_reset_fires_after_sustained_silence_robust`,
`idle_reset_does_not_fire_during_speech_robust`,
`idle_reset_disabled_when_cadence_zero_robust`.

---

## 2. Symptom #2 — breath / "stanza" pauses end the turn = endpointing

### Root cause

Both engines end an utterance after a **fixed tail-silence hangover** (default
`JFC_VAD_SILENCE_MS = 1000 ms`). Any within-utterance pause longer than that —
a breath, a mid-thought pause, a hesitation — trips `SpeechEnd`.

This is the **well-documented core weakness of silence-based endpointing**, not
a bug unique to us:

- **Skantze / Ekstedt & Skantze, *Voice Activity Projection* (arXiv:2205.09812),
  §1:** "silences *inside* of a speaker turn are often longer than silences
  *between* turns. Thus, [threshold] policies will either result in
  interruptions or sluggish responses." This is the precise mechanism behind
  "it cut me off when I paused."
- **Shi et al., *Semantic VAD* (arXiv:2305.12450):** traditional VADs "wait for a
  continuous tail silence to reach a preset maximum duration (e.g. 700 ms)
  before… segmentation," which both adds latency and mis-splits sentences. Their
  fix is a **semantic** breakpoint predictor (punctuation), shortening tail
  silence to ~300 ms after a complete thought and keeping the full ~700 ms
  otherwise.
- **Popit et al., *Thai Semantic End-of-Turn Detection* (arXiv:2510.04016):**
  "traditional audio silence end-pointers… fail under hesitations." They detect
  turn-completion from the **transcribed text**, not silence.
- **Chang et al., *Turn-Taking Prediction for Natural Conversational Speech*
  (arXiv:2208.13321):** a turn-taking predictor on top of an E2E recognizer
  reaches 97% recall / 85% precision at ~100 ms latency by combining acoustic +
  language cues, instead of a flat silence timer.

### Why this is a *feature*, not a one-line tweak

The real fix is a **semantic end-of-turn model** (text- or audio-based) that
holds the turn open mid-sentence and closes it fast after a complete thought.
That is a substantial new capability with its own model, latency budget, and
eval surface. Per the repo's scope rules it is **deliberately out of scope** for
a bugfix pass; see §6.

### Scoped fixes now (`neural_vad.rs`, parity with `vad.rs`)

1. **Anti-truncation min-speech guard** (the neural engine was *missing* the
   guard the energy VAD already had): require `JFC_VAD_MIN_SPEECH_MS`
   (default 200 ms) of captured speech before any `SpeechEnd` is honored, so a
   single stray low-probability chunk right after onset can't truncate the
   *start* of a sentence.
2. **Forgiving, documented, tunable hangover:** the 1 s default is intentionally
   generous to ride through normal pauses; it's now documented as the
   breath/stanza tradeoff and tunable via `JFC_VAD_SILENCE_MS`. Increase it if
   you take long thinking pauses; decrease it for snappier turn-ends.

Regression tests: `breath_pause_below_hangover_stays_one_segment_robust`,
`real_silence_still_ends_utterance_normal`,
`min_speech_guard_blocks_immediate_end_robust`,
`within_utterance_pause_stays_one_segment_robust`.

### Human reference points (why ~1 s is reasonable, and why semantics matter)

- **Stivers et al., *Universals and cultural variation in turn-taking*
  (PNAS 2009):** across ten languages, between-turn gaps cluster around a modal
  ~**200 ms**, with culture-specific means — humans target a *short* gap.
- **Levinson & Torreira (2015), "Timing in turn-taking":** because articulation
  planning takes ≫200 ms, listeners must **predict** turn ends from semantic +
  syntactic + prosodic content *before* the silence — they do not wait on
  silence. This is the cognitive argument for semantic endpointing over a timer.

---

## 3. Symptom #3 — background movie/TV voices detected = wrong problem class

### Root cause (a capability limit, not a bug)

Both engines answer **"is there human speech in this frame?"** Silero is trained
to fire on *any* human speech; the energy VAD's periodicity/modulation/flatness
gates reject *non-speech* noise (fans, AC, chair scrapes) but **not competing
speech**. A voice from a movie is real, modulated, periodic, harmonically
structured speech — by design it passes.

No **single-channel** VAD (energy or neural) can separate *your* voice from a
*background talker* at similar loudness/pitch. That is a different problem:
**target-speaker / primary-speaker extraction**, a.k.a. **Background Voice
Cancellation (BVC)**.

### What Krisp actually does

Krisp ships **two separate** neural capabilities:

- *Noise Cancellation* — removes non-speech noise (the part our gates already
  approximate), and
- *Background Voice Cancellation (BVC)* — a **separately trained
  source-separation model** that keeps only the **primary/near-field speaker**
  and suppresses other human voices (TV, open office, a second person).

BVC is **not a VAD**. It's speaker-conditioned separation. The relevant research
lineage:

- **Auditory scene analysis / the "cocktail party" problem** — Shinn-Cunningham
  & Best; Shamma et al., *Temporal coherence and attention in auditory scene
  analysis* (TINS 2011): the brain segregates streams by **temporal coherence**
  of features and **selective attention**, phase-locking cortical responses to
  the *attended* talker (Ding & Simon 2012, PNAS). Wu et al. (arXiv:2410.17620,
  *q-bio.NC*) model stream segregation as competing neural-pathway dynamics.
- **Machine target-speaker extraction** — e.g. Xu et al., *Target Speaker
  Verification with Selective Auditory Attention* (arXiv:2103.16269); the
  "cocktail fork" three-stem separation (arXiv:2110.09958). These require a
  **speaker enrollment / reference** to know which voice is "yours."

### Recommendation

Implementing BVC is a large, model-bearing feature and **out of scope**. It's
now documented as a known limitation in `neural_vad.rs`. Practical mitigations
that need no new model:

- a **close-talking mic** (proximity raises your SNR over the background),
- **push-to-talk / hold** mode in noisy rooms (bypasses always-listening),
- raising the onset threshold / periodicity bar (helps for *quiet* background
  speech only).

---

## 4. "Is hardcoding thresholds the right approach?" — the intensity question

Your instinct is correct: **absolute energy thresholds are the fragile part**,
because your voice changes tone and level. The literature's answer is to lean on
features that are **invariant to loudness** and that distinguish *voiced speech*
from *white/broadband noise* by **structure**, not amplitude.

### What's already adaptive (good)

- **Adaptive noise floor** (`vad.rs:352`): an EMA of frame RMS over **idle**
  frames; the onset threshold is `noise_floor × margin`, so it self-calibrates
  to the room instead of a fixed number. *Verified empirically* that this EMA is
  **symmetric** — after a noisy idle stretch (e.g. AC kicks on) the floor decays
  back down when quiet returns (probe: 150 → 2999 → 150), so the energy engine
  does **not** have a long-idle ratchet/drift bug. (The drift bug was
  Silero-specific; §1.)
- **Hysteresis** (double threshold): high onset, lower offset — rides through
  mid-word dips.
- **Pre-roll** (`recorder.rs:464`): a ~200 ms ring buffer prepended on
  `SpeechStart`, so the leading consonant isn't clipped (Silero calls this
  `speech_pad_ms`).

### What's intensity-*invariant* already (the right idea)

- **Normalized autocorrelation periodicity** (`frame_periodicity`, `vad.rs:478`):
  the autocorrelation peak is **normalized by frame energy** (`acc / energy0`),
  so it is **independent of how loud you are**. Voiced speech is quasi-periodic
  (scores high); white/broadband noise is aperiodic (scores low). This is the
  same quantity Praat's HNR is built on:
  **HNR = 10·log₁₀( r / (1 − r) )**, where `r` is the normalized
  autocorrelation peak (Boersma 1993). Because `r` is amplitude-normalized, HNR
  detects voicing **regardless of vocal intensity** — directly addressing "my
  voice gets quieter/louder."
- **Spectral flatness / Wiener entropy** (`spectral_flatness`, `vad.rs:522`):
  geometric-mean ÷ arithmetic-mean of the power spectrum — a **ratio**, hence
  scale-invariant. Tonal/voiced ≈ 0.1–0.4; white noise ≈ 1.0. This is the rVAD
  (Tan et al. 2019) / Moattar–Homayounpour discriminator.
- **Energy modulation** (`energy_is_modulated`, `vad.rs:317`): coefficient of
  variation (stddev/mean) of recent RMS — again a **ratio**, scale-invariant —
  captures the ~4 Hz syllabic pulsing of speech vs. a steady fan.

### How a mic stream differs from white noise (your exact framing)

| Property | Your voice (any tone/level) | White / broadband noise |
| --- | --- | --- |
| Normalized autocorrelation peak (periodicity) | High (has a pitch period) | ~0 (no period) |
| Harmonic structure / HNR | High; energy at f₀ harmonics | Low; energy spread |
| Spectral flatness (Wiener entropy) | Low (peaky) | ≈1 (flat) |
| Energy modulation (CoV) | High (syllabic pulsing) | Low (steady) |
| Absolute energy | **unreliable** (varies with you) | unreliable |

The point: the **first four rows are amplitude-normalized**, so they hold
whether you speak loudly or softly. Energy (last row) is the only fragile one,
and it's already backstopped by the adaptive floor + the structural gates.

### Grounded directions to make it *less* hardcoded (candidate follow-ups)

These are documented options, not yet implemented — they would need eval
coverage before changing default behavior:

1. **Statistical / likelihood-ratio decisioning** instead of fixed margins —
   Sohn et al. (1999), *A statistical model-based VAD*: model speech-present vs.
   speech-absent as distributions and decide by likelihood ratio with a
   self-adapting threshold.
2. **Long-Term Spectral Divergence (LTSD)** — Ramírez et al. (2004): compare a
   long-term spectral envelope to the tracked average **noise** spectrum and set
   the decision threshold **as a function of the measured noise level**, instead
   of a constant. Robust at low SNR.
3. **Promote HNR to a first-class gate** — we already compute the normalized
   autocorrelation; deriving HNR = 10·log₁₀(r/(1−r)) and gating on, e.g.,
   HNR > 0 dB is a principled, **loudness-independent** voicing test that would
   reduce reliance on the energy margin.
4. **Per-speaker calibration** — a short enrollment to learn *your* pitch range
   and typical HNR, narrowing the periodicity/pitch search to your voice. This
   also starts the path toward target-speaker gating (the only real fix for §3).

---

## 5. Files changed

- `crates/jfc-voice/src/neural_vad.rs`
  - Module docs: recurrent-state-drift explanation; background-voice (BVC)
    limitation.
  - `idle_reset_chunks` + `idle_silent_chunks`: gated idle state reset
    (`JFC_VAD_NEURAL_IDLE_RESET_MS`, default 5000 ms).
  - `min_speech_chunks` + `speech_chunks`: anti-truncation guard
    (`JFC_VAD_MIN_SPEECH_MS`, default 200 ms) — parity with the energy VAD.
  - 7 new regression tests (idle reset fires/gated/disabled; min-speech guard;
    breath-pause stays one segment; real end still fires).

No recorder change was required: the idle wait-loop already calls
`detector.push()`, so the neural fix takes effect there.

---

## 6. Deliberately out of scope (separate follow-up work)

- **Semantic end-of-turn / turn-taking model** (§2) — the real cure for
  breath/pause cutoffs; needs a model + latency budget + evals.
- **Background Voice Cancellation / target-speaker extraction** (§3) — the real
  cure for background-TV detection; needs a separation model + speaker
  enrollment.
- **Statistical/LTSD/HNR-gate threshold replacement** (§4) — principled
  de-hardcoding; needs eval coverage before changing defaults.

---

## 7. Sources (downloaded to `/tmp/vadpapers`, PDF→text)

- Ekstedt & Skantze, *Voice Activity Projection* — arXiv:2205.09812
- Shi et al., *Semantic VAD: Low-Latency VAD for Speech Interaction* —
  arXiv:2305.12450
- Popit et al., *Thai Semantic End-of-Turn Detection* — arXiv:2510.04016
- Chang et al., *Turn-Taking Prediction for Natural Conversational Speech* —
  arXiv:2208.13321
- Wang et al., *Breathing and Semantic Pause Detection… Post-Exercise Speech* —
  arXiv:2509.15473
- Wu et al., *Holistic structure of neural pathways… auditory stream
  segregation* — arXiv:2410.17620
- Ramírez et al., *Efficient VAD using long-term speech information* (LTSD) —
  Speech Communication / ugr.es specom04
- *Voice Activity Detection: Fundamentals and Speech Recognition System
  Robustness* (2007 survey)
- Stivers et al., *Universals and cultural variation in turn-taking* —
  PNAS 2009
- Levinson & Torreira, *Timing in turn-taking and its implications…* —
  Front. Psychol. 2015 (PMC4464110)
- Shamma et al., *Temporal coherence and attention in auditory scene analysis* —
  TINS 2011; Ding & Simon, PNAS 2012 (cortical tracking of the attended talker)
- Cepstral Peak Prominence / Harmonics-to-Noise Ratio clinical literature
  (PMC4826073 and related) — intensity-independent voicing measures
- Boersma (1993), HNR via normalized autocorrelation —
  HNR = 10·log₁₀(r/(1−r))
- Sohn et al. (1999), *A statistical model-based VAD* — likelihood-ratio decision
- Silero VAD (`snakers4/silero-vad`) discussions/README on state reset; Pipecat
  `SileroVADAnalyzer` (`_MODEL_RESET_STATES_TIME = 5.0`); `voice_activity_detector`
  crate v0.2.1 source (recurrent `state` field carried across `predict()`).
