# Spotify preset/style contract — bounded recovered investigation, 2026-09-19

Scope: read-only inspection of existing Sep16 private bundle/descriptors/RPC records, current identical DLL, current loaded enum module, and 29 additional local native style RPCs on Sep19. No playback, playlist, authentication, or production changes. Source precedence is specified in [SPOTIFY_LIVE_RECIPE_PROTOCOL.md](SPOTIFY_LIVE_RECIPE_PROTOCOL.md). Exact fact/schema/ID tables and hashes: [`2026-09-19-style-contract.json`](../tools/runtime-diagnostics/evidence/2026-09-19-style-contract.json). Private source and assembly stay under `.codex/protocol-20260919/`.

Numeric request/response corpus: [`2026-09-19-style-responses.json`](../tools/runtime-diagnostics/evidence/2026-09-19-style-responses.json). It includes all69 completed calls; use these values, not hashes alone, for resolver fixtures.

## Evidence and confidence

- **PROVEN** means directly observed descriptor/wire field, response, or unambiguous native control/copy branch for the recorded version. **HIGH CONFIDENCE** means supported native semantic dataflow interpretation, not all runtime edge cases exercised. **MEDIUM/LOW** would denote weaker inference; **UNKNOWN** means not established.
- DLL `1.3.0.277`, SHA256 `65c131dc7f9dcce90eb3874033e6e22ef422a493f0877e3f031805198839b7c4` (same as prior extraction).
- Embedded mixing bundle SHA256 `c6d71a1b75bf5bc5ea0f33a1f2a5934c651b50d3dc054f2de75dfab01e6d8285`; Sep16 GetMixingBundle response wrapper SHA256 `13f02e6a00d562179edd1d9b919ee926fe0f426f9b58f2570985c0e288304772`.
- **PROVEN:** decoding that wrapper with extracted descriptors yields a mixingBundle identical to embedded JSON. Different hashes are wrapper-vs-inner bytes, not proof of different data.
- Old records: `.codex/protocol-20260916/styles.json` captured `2026-09-16T10:11:25.972Z`; new `.codex/protocol-20260919/style-rpc.json` captured `2026-09-19T10:12:34.837Z`, 29 successful calls, zero errors. These are diagnostic style calls, not user preview calls.
- Current enum module72481 source/exports are private `style-enum-sources.json`/`style-enums.json`; all source hashes are in sanitized evidence. Current zip contains `dwp-panel-section.js`; it lacks xpui-modules.js. Do not mix old extracted Aug13 Apps/xpui with Sep14 archive/current loaded modules.

## Service and wire contract

**PROVEN** full message/field tables are in sanitized evidence `schema` (machine-readable semantic facts, not source).

- Service `spotify.automix.esperanto.proto.Automix`, generated client method spelling lower camel; wire methods `GetStylesForPresetId`, `GetVolumeStylesForVolumeStyleId`, `GetEqStylesForEqStyleId`, `GetFilterFxStylesForFilterFxStyleId`, `GetFxStylesForFxStyleId`, `GetJogwheelBlocks`, `GetLoopingBlocks`.
- Preset request repeated int32 IDs field1; response repeated PresetIdStyle field1. Row fields: preset_id1, volume_style2, eq_style3, filter_fx_style4, fx_style5, jogwheel_style6, looping_style7. All Style wrappers have int32 id1.
- Volume/EQ/filter requests: repeated int32 IDs1, int32 num_bars2. Responses repeated family StyleCurves1.
- **Important:** FX request repeated int32 fx_style_ids1, **float bpm2**, **int32 num_bars3**. Old probe omitted BPM.
- Jogwheel request IDs1, num_bars2; looping request IDs1 only; both response repeated StyleBlocks1. StyleBlocks style1, out2, in3. BlockSequence repeated blocks1. StyleBlock enum type1, float beats2, float start3, float end4. Type0 unspecified,1 loop,2 spinback,3 vinyl-stop.
- VolumeStyleCurves style1/fade_out2/fade_in3; EQ style1/low_out2/mid_out3/high_out4/low_in5/mid_in6/high_in7; filter style1/cutoff_out2/resonance_out3/cutoff_in4/resonance_in5/noise_out6/noise_in7; FX style1/echo_out2/echo_in3/reverb_out4/reverb_in5/delay_out6/delay_in7.
- Point double x1/y2. AutomationCurve repeated points1/double start2/end3. **Transition/service AutomationCurves curves1/minimum3/maximum4; bundle AutomationCurves curves1/minimum2/maximum3.** Never reuse one wire message for both.
- Native source descriptors are proto3, whereas repository reconstructed `automix_transition.proto` intentionally uses proto2 optional presence. Wire numbers agree for represented fields, but default/presence behavior must not be assumed identical. Repo lacks service response/blocks/bundle schemas.
- AutomixBundle.GetMixingBundle has empty request and response mixing_bundle1; complete bundle fields1..15 in evidence. No version/revision field. Identifier id1/string_id2/sales_period3; SalesPeriod int64 start1/end2, time unit UNKNOWN.
- GetMaxTransitionLength and GetBeatmatchOptOutTransitionDuration return int32 milliseconds1, observed35000/5000 respectively.

## IDs and preset expansion

**PROVEN** current enum export tables in evidence are complete *for that module*, not a universal future allowlist. Unknown=-1 is a UI sentinel; numeric unknowns can still receive fallback RPC output.

- Presets0..22: None,SimpleFade,EqSwap,BassSwap,BassSwapFilterIn,BassSwapFilterOut,BassSwapEcho,BassAndMidSwap,FilterFadeOut,FilterFadeIn,FilterFadeInOut,SimpleCut,ReverbCut,RiserCut,ReverbRiserCut,EchoCut,BassSwapAtEnd,BassSwapLowPassFilterIn,BassSwapLowPassFilterOut,LowPassFilterFadeInOut,Dissolve,Surge,Afterglow.
- Volume0..11: None,Cut,Crossfade,SlowInFastOut,FastInSlowOut,SlowInSlowOut,CrossShape,FullOverlap,Switcheroo,SlowInFastOutAtEnd,FastInAtStartSlowOut,SlowInSemiFastOutAtEnd.
- EQ: 0None,1EqSwap,2BassFadeIn,3BassCrossfade,4BassSwap,7BassAndMidSwap,8MidSwap,9HiSwap,10BassSwapAtEnd,11BassSwapAtStart,12BassCuts,13NoBass,14StartFadeOut,15BassFadeOut. IDs5/6 not in enum.
- FilterFx0..15: None,Echo,Reverb,Riser,ReverbRiser,FilterIn,FilterOut,FilterInFilterOut,EchoAtEnd,LowPassFilterIn,LowPassFilterOut,LowPassFilterInLowPassFilterOut,FilterInLowPassFilterOut,LowPassFilterInFilterOut,HighPassFilterHalfOut,NoiseOutEnd.
- FX0..11: None,ReverbOutCentre,ReverbCutEnd,ReverbOutEnd,EchoHalfCutEnd,EchoHalfOutEnd,Echo3QCutEnd,Echo3QOutEnd,DelayHalfCutEnd,Delay3QCutEnd,Echo1CutEnd,Echo1OutEnd.
- Looping: 0None,1TwoBeat,2FourBeat,3EightBeat,4SixteenBeat,5HalvingEnd,8OneBeat. Jogwheel is absent from this enum module; observed native IDs detailed below.
- All preset0..22 expansions are in evidence `preset_to_styles`. Key newly closed gaps:20Dissolve=(volume9,EQ10,filter14,FX5);21Surge=(9,10,14,3);22Afterglow=(6,4,0,1). All observed presets have jogwheel0,looping0.
- Bundle lists only six presets1,3,2,17,4,11. It is not the complete native preset universe. Its legacy volume/EQ/filter mappings are in evidence; bundle FX lists absent. All directional style Identifier numeric IDs are omitted (proto3 defaults0), though definitions/mappings reference positive side IDs. **UNKNOWN:** exact side-ID assignment routine; do not silently key these entries by omitted id or assume positional numbering without further proof.

## numBars, BPM, item speed

- **PROVEN:** old RPC responses at numBars0,1,2 are identical; curves for volume1,7,9,10 and EQ1,4,7,10 vary at3,4,8,16,32. Volume0,5,6 and sampled filter/FX envelopes remain identical across old sampled bars. Current full-family responses at2/4/8 are summarized/hashes retained.
- **PROVEN native:** materializer0x01040aa4 clamps bars to [2,32], 0x01040b2e..0x01040b50; constants0x01aa9ea0=2 and0x01aa9ea4=32.
- Volume Cut at4bars: outgoing flat1 until0.5, falls to0 by0.515625; incoming rises0->1 over0.484375..0.5. Observed width is1/(16*N) for tested N>=2; smooth segments use two or four points. Do not quantize RPC bars to bundle's discrete0/2/4/8/16/32 entries: native3bar output exists.
- **PROVEN:** current FX IDs0..11 responses exactly identical at BPM0,60,120,240 with4bars, and at2/4/8bars with120BPM. These returned messages describe wet/dry envelopes only. **UNKNOWN:** effect timing/filter internals cannot be inferred BPM-independent.
- Current UI enables FX curve query only for nonzero style and BPM>0, query cache includes style/BPM/bars, passes effectiveFxBpmA; display behavior is distinct from native API accepting BPM0.
- **HIGH CONFIDENCE native:** materializer resolves optional item_speed_a/b to1. For each side positive supplied BPM is used; otherwise clampedBars*240/outgoingDurationSeconds if positive, else120. Effective BPM=resolvedBPM*itemSpeedSide. Incoming fallback also uses outgoing duration. Evidence0x01040af4..0x01040bd0; constants0x01aaa27c=240f,0x01964478=120f,0x01902be0=1d.
- **HIGH CONFIDENCE native:** durations/positions become ms with factor1000 at0x01042e62..0x01042ecc. Beatmatched incoming speed helper0x01047af8 receives incomingDuration/outgoingDuration*itemSpeedA plus targetItemSpeedB (0x01042ed3..0x01042f1c). Nonbeatmatched branch emits single speed point at0 for itemSpeedB differing from1 beyond epsilon (0x01042f5e..0x01042f9d). Exact ramp helper not inspected here; the main protocol specification records the recovered speed helper. Do not conflate item speed with overlap speed fields.

## Curve coordinates, units, and omissions

- **PROVEN data:** curve segment start/end and point x/y values are normalized in observed output (0..1); segment start/end span envelope, x is local segment coordinate. UI editor emits local x0,1/3,2/3,1 for cubic controls and0,1 for lines, global start/end positions; it clamps editable values0..1. **HIGH CONFIDENCE:** two/four points represent line/cubic Bezier; actual renderer evaluation/inversion still UNKNOWN from this subtask.
- Service returned minimum=maximum=0 for examined channels despite varying y. UI-created override envelope instead sets minimum0/maximum1. Treating zero/zero as a scale yielding silence is demonstrably wrong.
- **PROVEN native:** override conversion0x01037958..0x01037b92 copies each curve's points/start/end unchanged, ignores envelope minimum/maximum entirely; no channel conversion/clipping happens in that function.
- **HIGH CONFIDENCE:** y is a channel control coordinate, not one universal physical unit. EQ neutral-looking flat curves and filter center are0.5; filter directional sweeps go0.5->0 or1, whereas volume uses0/1. Exact normalized-y to amplitude/dB, cutoffHz, resonanceQ, noise color, wet/dry law remains **UNKNOWN**. This is a production renderer blocker; labels and endpoint shapes alone are insufficient.
- **PROVEN missing/invalid RPC behavior:** empty ID list returns empty response list. IDs[-1,999,0] preserve order and requested IDs; invalid volume returns simple linear0/1 fades; volume0 returns *nonempty shaped* fades, unlike other family0 emptiness. Invalid/0 EQ and filter return all named CurveSets present but empty, min/max0. Invalid/0 FX returns only style wrapper, omitting curve fields. Negative/999 looping and jogwheel999 return style only. No error was thrown for these requests.
- **UNKNOWN:** malformed raw protobuf, NaN/nonfinite, transport failure, partial/mismatched arrays, missing schema fields and arbitrary future invalid values were not tested. Client implementation should distinguish absent, empty, default, and invalid; do not turn RPC fallback success into an allowlist/validity proof.

## Override precedence and native/UI distinction

- **PROVEN native0x01043b9c:** validate preset through manager before expansion. Unknown preset gives no materialized result. Preset0NONE produces no transition before overrides; any overlap/styles ignored. This differs from getStylesForPresetId(-1/999) returning volume6+other0, echoing invalid preset ID.
- Valid nonzero preset expands styles, then six present ID override messages replace volume/EQ/filter/FX/jogwheel/looping IDs, including explicitly0. Curve extraction runs only if config byte+0xa0 enabled: `core-automix/auto_transition_use_curve_overrides`, accessor0x0048163c, compiled default false, stored at+0xa0 by0x00482475; effective account override remains UNKNOWN. Source locations0x01043cc3..0x01043df0; mapping verified against extracted proto offsets/presence bits.
- **PROVEN:** converter0x01043264 retains group and child optional presence separately. Volume present override replaces that side's curves, even empty, else queries selected style (0x01040bfd..0x01040d56).
- **HIGH CONFIDENCE:** for native EQ/filter, a present side override group bypasses that entire side's default group; absent child becomes empty vector. Example EQ-out group test0x01040d56, absent-low0x01040dcb, defaults only branch0x010412ed. EQ-in analogous0x0104142e..0x0104163f. Filter-out analogous group/default branches up to0x01041ada; filter-in follows. Thus partial sibling omission is not necessarily per-channel fallback.
- **PROVEN:** FX style0 causes FX override ignore/log branch0x01042e2d..0x01042e5d. Nonzero FX resolves base effect parameters and only substitutes explicitly present dry/wet channels (0x010428b1..0x01042905 and0x01042dd7..0x01042e26); hidden FX parameter curves remain generated.
- UI panel uses nullish per-channel fallback for EQ/filter/FX display and can render absent siblings from base style. Do not elevate display merge to native playback specification. **UNKNOWN:** native group-clearing behavior with an actual user-authored partial override has not been end-to-end exercised; use synthetic native materialization oracle next, not production code guesswork.

## Jogwheel and looping relevance

- **PROVEN:** response schemas and preset override fields15/16 mean these are real contract families, not names to silently discard. Native materializer has nonzero jogwheel and looping branches0x01042fa2..0x010430c9. None of queried standard preset0..22 enables them by default; explicit overrides remain relevant.
- Jogwheel2/4/8bar probes: IDs1/2/3 emit outgoing spinback blocks of1/2/4beats, ending1 and starting1-beats/(4*N). ID4 vinyl-stop1beat spans1-1/N ..1-3/(4*N); ID5 spans1-1/(2*N)..1-1/(4*N); ID6 spans1-1/(4*N)..1; ID7 vinyl-stop4beats spans1-1/N..1. IDs0,8,999 emit no blocks. Names beyond observed type/timing **UNKNOWN**; no claim IDs>8 universally unsupported.
- Looping1/2/3/4 emit outgoing loop2/4/8/16beats across0..1. ID8 loop1beat across0..1. ID5 halving-end emits1beat0.5..0.75,0.5beat0.75..0.875,0.25beat0.875..1.0. ID0/unknown omitted out/in; no incoming blocks observed. Full block responses retained as semantic data.
- **UNKNOWN:** exact looping/spinback/vinyl-stop DSP, source repeat position, stop/release behavior, limits and interactions with speed/overlap ownership. They must be rejected/explicitly unsupported by any initial renderer rather than approximated invisibly.

## Static/fetched/versioning conclusion and next action

- **PROVEN:** static DLL resource and Sep16 native bundle RPC agree exactly, providing an offline data source. Current native RPC covers more styles/presets than bundle. No explicit bundle version field is present. **UNKNOWN:** whether any runtime fetch/replacement mechanism exists; equality does not establish that none exists. Pin evidence to DLL SHA/client version/descriptors and current enum source hashes, not marketing names or implicit eternal IDs.
- **PROVEN implementation-ready:** wire schema, enum IDs, observed preset mapping, response omissions/fallbacks, bars/BPM request dimensions, explicit NONE/unknown distinction, gate-dependent native override precedence, block shapes.
- **Remaining semantic blockers:** physical DSP y mappings and evaluator; hidden FX parameter materialization; exact side-ID assignment; effective config flag controlling curve overrides; partial-group override runtime confirmation; effect/block DSP and bundle update policy. Do not claim a full compatible renderer from the now-complete style lookup wire contract.
- Implementation order and release gates are in the main protocol specification. A capability-gated semantic resolver is implementable now; physical channel mappings remain required before claiming complete DSP compatibility.
