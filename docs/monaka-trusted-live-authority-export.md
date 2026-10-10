# Monaka 5AI-S accepted authority export

This fork phase starts from authority prototype
e94ff1967c70da480531c3ec9f6956d3be8b5ce5 and upstream
e0d83b46168449cb0bb514770dd926b0ed85c55a. The real source is the existing
Android OpenXR VIEW-in-STAGE evidence captured before server recenter/prediction.
Generic ALVR tracking is not authority. Missing, ambiguous, stale, conflicting or
retired evidence cannot become a trusted source sample.

Export is disabled unless the ALVR server process has an explicit local
`MONAKA_ALVR_AUTHORITY_PIPE=\\.\pipe\<operator-registered-name>` environment value.
No device, service, firewall, registry or system-wide environment is changed here.
The server is a local pipe client, not a listener. It sends only TrackingManager
Accepted evidence after the existing single-HEAD, exact pose-bit and replay check.
An unavailable server pipe does not cause a fixture fallback or a tracking queue.

The internal schema `alvr-openxr-authority-export-1` sends `connected`, immutable
`sample`, `disconnect`/`authority-lost`, `orderly-shutdown` or `stream-failure`.
The seven source float bit patterns and all six validity flags are unchanged.
Observation/reference generation and signed OpenXR locate evaluation nanoseconds
are decimal strings. Locate time is runtime monotonic evaluation, not physical
capture or wall-clock time; its epoch/resolution are runtime-defined. Position is
metres in right-handed STAGE (+X right,+Y up,-Z forward); quaternion is Hamilton
xyzw. A source token names the OpenXR authority/clock lifetime and survives an
ordinary network reconnect. Each export connection has a fresh OS-random 128-bit
session, separate from that token. Export time is elapsed monotonic nanoseconds
since exporter creation. The current full Git commit is embedded at compile time
and reported as `exporter_build_commit`.

Each message is bounded to 2048 bytes, with nonblocking all-or-none writes to the
converter. Steady accepted raw observations are thinned before publication
sequence allocation to a 50 ms minimum export interval (20 Hz cap), leaving
headroom for A2's existing 20 ms control polling. Original observation IDs, source
times and poses remain exact; `export_sample_skip_count` records every omitted
observation and `export_min_sample_interval_ms` reports 50. Definition/reference,
validity/lost changes and reconnect are immediate. Generic ALVR tracking remains
at its original rate. The zero-interval seam exists only in software tests to
exercise saturation without arbitrary sleeps.

Nonblocking all-or-none writes target the
converter's 8192-byte local message pipe. No application queue exists. Failed or
short writes close the connection and count drops; duplicate/rejected accepted
stream inputs count rejects. Reconnect requires new accepted evidence and a new
export session. Shutdown attempts an explicit lifecycle message before close.

Canonical installation identity is configured at the MonakaBridge converter as
operator-registered `source_publisher_id`; it is not derived from PID, this pipe,
export/OpenXR session, sequence, owner epoch or transport. The converter encodes
the existing production monaka.hmd_authority wire 2.1 contract. The Bridge adapter
uses A2's existing inherited control stream and its explicit authorization gates;
the existing publisher carries identity through wire 2.2 to the R2 association.
See MonakaBridge `docs/trusted-live-source-a2-adapter-v2.1.md` for the full contract.

Software tests cover exact source bits, actual Windows message-pipe writes from
TrackingManager's real acceptance path, duplicate rejection, reconnect fencing
and bounded overflow. They supply deterministic test evidence and do not start
ALVR/SteamVR, connect a physical headset, install the DLL or perform HIL. The
Release Windows driver artifact is a real rebuilt producer component; real use
requires a matching authority-capable Android OpenXR client from this lineage,
its OpenXR runtime and a compatible ALVR/SteamVR installation. Environment
re-binding (5AH-E2) and HIL (5AH-B) remain separate work.
