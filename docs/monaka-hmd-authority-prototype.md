# Monaka Phase 2B-5X: OpenXR HMD authority prototype

**PROTOTYPE PROTOCOL ONLY.** Based on alvr-org/ALVR
`e0d83b46168449cb0bb514770dd926b0ed85c55a`, 21.0.0-dev14, MIT.
Patched client and server must be paired. No Monaka IPC or production admission
is implemented. No driver or APK installation is part of this prototype.

## Contract and acquisition

The source boundary is one `XrSession` and the runtime's VIEW-relative-to-STAGE
reference contract. A process-wide non-reuse registry issues a 128-bit OS-random
token once per new XrSession. Stream reconnects and reference-space recreation
do not issue new tokens. Counters start at zero and range through INT64_MAX.
Entropy failure disables authority while retaining ordinary tracking.

Khronos OpenXR 1.1.63 defines the reference change boundary at requested locate
time **greater than or equal to changeTime**. The pending queue uses that exact
boundary, including equality, rather than receipt time. STAGE and VIEW are
relevant. LOCAL and LOCAL_FLOOR_EXT are irrelevant to this fixed capture pair;
unknown spaces conservatively invalidate authority. Equal or decreasing event
times, decreasing sample times, generation overflow, or pending queue overflow
invalidate it. poseValid=false does not remove the boundary; continuity mapping
from the previous origin remains unavailable.

Sources (accessed 2026-10-09):
[ReferenceSpaceChangePending](https://registry.khronos.org/OpenXR/specs/1.1/man/html/XrEventDataReferenceSpaceChangePending.html),
[EventsLost](https://registry.khronos.org/OpenXR/specs/1.1/man/html/XrEventDataEventsLost.html).
EventsLost permanently invalidates the current authority session; its count does
not reconstruct generations. Instance/session loss retires authority. STOPPING
stops evidence emission and invalidates the baseline; rebegin on the same
XrSession cannot restore it. A new XrSession is required. These transitions only
close authority; existing generic ALVR tracking/lifecycle behavior is preserved.

`get_head_data()` copies the original `head_location.pose` without normalization
or substitution and returns six raw validity/tracked facts. The existing
future locate on PICO/VIVE/YVR estimates velocity; it is never the source pose.
Failed acquisition emits no observation. `source_locate_time` is the **now**
passed to the head relate/locate_views calls. `poll_timestamp` remains the
existing **target_time**, after the client prediction offset. Neither is a
sensor physical acquisition timestamp.

## Concurrency and transport

The event handler records its boundary before the original stop/recreate/join/
restart reference-space barrier. Input acquisition holds the existing interaction
read lock but never the authority gate. After every OpenXR call and after dropping
that lock, the input thread holds the authority mutex across evidence issuance
and same-TrackingData send. Event handling uses the same mutex. If an event is
received after a sample at or beyond its boundary, the session invalidates;
already-issued samples are never relabeled. Failed sends may conservatively
count as issued for the late-event gate; this can close authority earlier but
cannot create authority. No OpenXR call holds the authority gate.

The client and the decoded server packet both compare all seven source P/Q
components with `f32::to_bits()`, and require exactly one HEAD_ID. Malformed
binding drops only `hmd_authority`. Ordinary motion, timestamps, controllers and
other tracking fields remain. All non-OpenXR TrackingData constructors emit None.

The packet roundtrip test measures bincode's actual standard encoding: the
original five-field HEAD-only fixture is 71 bytes, patched None is 72, patched
Some with small counters is 125, and Some with INT64_MAX counters/time is 149.
Added overhead versus upstream is 54 to 78 bytes/sample (None adds 1).
At an assumed 90 Hz refresh and the existing three samples/frame input cadence,
270 samples/s would add 14,580 to 21,060 bytes/s, or 116.64 to 168.48 kbit/s.
These are calculated payload increments, not measured runtime rates or wire
bandwidth; packet framing and non-HMD contents are excluded.

## History, mapping, prediction

Each MotionHistorySample contains the actual tracking timestamp, transformed
motion, immutable source evidence, and the recenter mapping/epoch applied when
that entry was inserted. Exact/nearest-newer lookup returns that same entry.
For 100/A and 110/B, query 105 returns motion B, evidence B and timestamp 110.
The motion-only wrapper preserves the original behavior.

The receiver retains per-source-session high-water across network reconnects.
Lower IDs and generation decreases reject authority. Identical high-water
payloads are duplicates; changed immutable payloads at the same ID retire the
session and revoke even stored history evidence. Fresh tokens start a new
high-water; a switched-away or retired token cannot revive. Source pose/flags are
never modified by recenter, velocity scaling, offsets or prediction.

Recenter epoch advances only when the exact seven mapping float bits change.
The snapshot and epoch remain with each history entry, including old entries
selected after a later recenter. Epoch overflow disables evidence only.
This covers **ALVR internal recenter mapping only**. It does not cover SteamVR
standing/world/chaperone or Monaka common-space transformations.

HMD prediction intentionally retains upstream behavior: from the original
event/query poll_timestamp to the motion-to-photon target. Both are recorded
independently of the actual selected history timestamp. Source evidence remains
unchanged when prediction changes the provider output.

## Typed provider boundary

FfiHmdAuthorityEvidenceV1 is 144 bytes, alignment 8. It uses fixed-width integers,
token bytes, source floats, raw validity mask, selected timestamp, prediction
from/target, recenter epoch and mapping snapshot. No new bool ABI is used.
Unrepresentable Duration values discard evidence instead of truncating it.

SetTracking passes the DTO to Hmd::OnPoseUpdated. The latter retains an association
with the exact DriverPose_t double-valued P/Q actually submitted. A submit gate
associates a selected observation at most once; repeated selection still submits
the ordinary pose with evidence present=0. Unselected observations need not have
a provider update. Every active Hmd::OnPoseUpdated call retains the single existing
submit_pose -> TrackedDevicePoseUpdated call. No extra SteamVR update is added.
Inactive HMD behavior remains zero submissions.

Upstream provider poseIsValid=true, Running_OK, deviceIsConnected=true are
unchanged and are **not OpenXR validity**. No Strong Trusted sourceValid is made.

PC diagnostics are off unless `MONAKA_ALVR_HMD_AUTHORITY_TRACE=1`.
Prefix: `MONAKA_ALVR_HMD_AUTHORITY_V1`. Provider diagnostics include source float
bits, exact provider double bits and identity/provenance. Client acquisition/
reference diagnostics use debug logging. Text traces are never an authority input.

## Validation and reproduction

Required Rust gates:

```text
cargo fmt --check
cargo test -p alvr_common
cargo test -p alvr_packets -- --nocapture
cargo test -p alvr_server_core
cargo test -p alvr_server_openvr
cargo check -p alvr_client_openxr
cargo clippy -p alvr_common -p alvr_packets -p alvr_server_core -p alvr_server_openvr -p alvr_client_openxr --all-targets
```

On Windows the OpenVR and libvpl runtime DLL directories must be on PATH for
server_openvr tests. Bindgen requires libclang. To generate a cross-language
fixture, set MONAKA_ALVR_AUTHORITY_FIXTURE_DIR to a writable directory before
running the FFI test. Then run:

```text
python alvr/server_openvr/cpp/tools/test_hmd_authority.py <fixture-dir>/rust-ffi.bin
```

The C++ runner compiles the exact production SetTracking, Hmd::OnPoseUpdated,
and TrackedDevice::submit_pose bodies, emitting SHA256s. It substitutes only
unrelated driver construction/video/controllers and implements the real SDK
IVRServerDriverHost with a fake host. It checks exact source/ABI bytes, provider
double bits, one submit per active call, duplicate/conflict/switch gates and
generic fallback. It neither loads nor registers a SteamVR driver. Full streamer
compilation is a separate gate. MONAKA_TEST_VS_ROOT and MONAKA_TEST_SDK_ROOT can
override this workspace's MSVC/Windows SDK locations.

Build commands are prepare-deps --platform windows --ci (skip system package
installation), build-streamer --platform windows, and build-client via cargo
xtask. No --gpl flag is used. APK validation requires cargo-apk plus Android
SDK/NDK; host client checking does not establish an APK or physical result.

## Future seam and one next phase

Source evidence could later fan out immediately after decoded packet validation
and receiver acceptance, or from the actual selected history entry immediately
before prediction. A fanout must share that value and entry, with no reread or
second locate. This phase adds no IPC.

Architecture A, direct immutable OpenXR source input to Monaka, is the preferred
future candidate: source identity/space/flags are clearer and do not inherit
provider prediction and fixed validity. Architecture B, provider output as input,
has an exact association in this prototype, but additionally requires reviewed
prediction/time, output-validity and SteamVR world mapping contracts. Neither
qualifies current RawHmdPoseInput or Strong Trusted.

Selected next phase: **Phase 2B-5Y — ALVR/OpenXR→Monaka Mapping Authority Design**.
The source chain alone does not establish the complete STAGE -> ALVR provider ->
SteamVR standing/world -> Monaka common-space mapping needed for meaningful
coordinate comparisons. Physical HIL remains a separately authorized future task.

Monaka remains Strong Trusted UNSUPPORTED, RawHmdPoseInput BLOCKED, Position
Correction NOT ENABLED, 2B-5P NOT READY. 5S remains BLOCKED — PHYSICAL PROBE
DEPLOYMENT / STEAMVR RESTART DEFERRED. License and attribution notices are retained.
