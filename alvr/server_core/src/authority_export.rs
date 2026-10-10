//! Real accepted, pre-transform OpenXR evidence only. Disabled unless explicitly configured.
//! Windows local message pipe client, nonblocking writes, no queue and no fixture fallback.
use alvr_common::hmd_authority::{HmdAuthorityEvidenceV1, pose_bits};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub(crate) fn sample_frame(e: HmdAuthorityEvidenceV1) -> Value {
    json!({
        "schema": "alvr-openxr-authority-export-1", "event": "sample",
        "source_token": e.session_epoch.0.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "observation_id": e.observation_id.to_string(),
        "reference_generation": e.reference_space_generation.to_string(),
        "source_locate_time_ns": e.source_locate_time.to_string(),
        "source_pose_bits": pose_bits(e.source_pose), "validity_mask": e.validity.bits()
    })
}

pub(crate) struct AuthorityExport {
    pipe: String,
    started: Instant,
    handle: Option<platform::Pipe>,
    session: Option<String>,
    pub rejected: u64,
    pub dropped: u64,
    reconnects: u64,
    last_failure: Option<&'static str>,
    min_interval: Duration,
    last_sample: Option<(HmdAuthorityEvidenceV1, Duration)>,
    skipped: u64,
}
impl AuthorityExport {
    #[cfg(test)]
    pub(crate) fn test_pipe(pipe: String) -> Self {
        Self {
            pipe,
            started: Instant::now(),
            handle: None,
            session: None,
            rejected: 0,
            dropped: 0,
            reconnects: 0,
            last_failure: None,
            min_interval: Duration::ZERO, // explicit test-only saturation seam
            last_sample: None,
            skipped: 0,
        }
    }
    pub fn configured() -> Option<Self> {
        let pipe = std::env::var("MONAKA_ALVR_AUTHORITY_PIPE").ok()?;
        // Explicit local name only. No UNC host, path traversal or implicit endpoint.
        let name = pipe.strip_prefix(r"\\.\pipe\")?;
        if name.is_empty()
            || name.len() > 96
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        {
            return None;
        }
        Some(Self {
            pipe,
            started: Instant::now(),
            handle: None,
            session: None,
            rejected: 0,
            dropped: 0,
            reconnects: 0,
            last_failure: None,
            min_interval: Duration::from_millis(50),
            last_sample: None,
            skipped: 0,
        })
    }
    fn send(&mut self, mut value: Value) -> bool {
        value["export_session"] = json!(self.session);
        value["exporter_build_commit"] = json!(env!("MONAKA_ALVR_BUILD_COMMIT"));
        value["export_time_ns"] = json!(self.started.elapsed().as_nanos().to_string());
        value["export_reconnect_count"] = json!(self.reconnects);
        value["export_drop_count"] = json!(self.dropped);
        value["export_reject_count"] = json!(self.rejected);
        value["export_sample_skip_count"] = json!(self.skipped);
        value["export_min_sample_interval_ms"] = json!(self.min_interval.as_millis() as u64);
        value["last_stream_failure"] = json!(self.last_failure);
        let Ok(bytes) = serde_json::to_vec(&value) else {
            return false;
        };
        self.handle
            .as_ref()
            .is_some_and(|h| bytes.len() <= 2048 && h.write(&bytes))
    }
    pub fn observe(&mut self, evidence: HmdAuthorityEvidenceV1) {
        let at = self.started.elapsed();
        // Thin accepted raw observations BEFORE publication sequence allocation.
        // No pose modification/queue. Definition and validity changes are immediate.
        if self.handle.is_some() && !self.sample_due(evidence, at) {
            self.skipped = self.skipped.saturating_add(1);
            return;
        }
        if self.handle.is_none() {
            let Some(handle) = platform::Pipe::open(&self.pipe) else {
                self.dropped = self.dropped.saturating_add(1);
                self.last_failure = Some("pipe-unavailable");
                return;
            };
            let Some(session) = platform::fresh_session() else {
                return;
            };
            self.handle = Some(handle);
            self.session = Some(session);
            self.reconnects = self.reconnects.saturating_add(1);
            if !self.send(json!({"schema":"alvr-openxr-authority-export-1","event":"connected"})) {
                self.fail("connect-write-failed");
                return;
            }
        }
        if !self.send(sample_frame(evidence)) {
            self.dropped = self.dropped.saturating_add(1);
            self.fail("backpressure-or-pipe-failure");
        } else {
            self.last_sample = Some((evidence, at));
        }
    }
    fn sample_due(&self, e: HmdAuthorityEvidenceV1, at: Duration) -> bool {
        self.last_sample.is_none_or(|(old, time)| {
            old.session_epoch != e.session_epoch
                || old.reference_space_generation != e.reference_space_generation
                || old.validity != e.validity
                || at
                    .checked_sub(time)
                    .is_some_and(|delta| delta >= self.min_interval)
        })
    }
    pub fn reject(&mut self) {
        self.rejected = self.rejected.saturating_add(1);
    }
    fn fail(&mut self, reason: &'static str) {
        self.last_failure = Some(reason);
        self.handle = None;
        self.session = None;
    }
    pub fn disconnect(&mut self, event: &'static str) {
        if self.handle.is_some()
            && !self.send(json!({"schema":"alvr-openxr-authority-export-1","event":event}))
        {
            self.last_failure = Some("lifecycle-write-failed");
        }
        self.handle = None;
        self.session = None;
    }
}
impl Drop for AuthorityExport {
    fn drop(&mut self) {
        self.disconnect("orderly-shutdown");
    }
}

#[cfg(windows)]
mod platform {
    use std::{ffi::c_void, ptr};
    type Handle = *mut c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            sa: *const c_void,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn SetNamedPipeHandleState(
            handle: Handle,
            mode: *const u32,
            count: *const u32,
            timeout: *const u32,
        ) -> i32;
        fn WriteFile(
            handle: Handle,
            bytes: *const u8,
            len: u32,
            written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(algorithm: Handle, bytes: *mut u8, len: u32, flags: u32) -> i32;
    }
    pub struct Pipe(Handle);
    // Owned by the TrackingManager's existing serialization gate, never shared.
    unsafe impl Send for Pipe {}
    unsafe impl Sync for Pipe {}
    impl Pipe {
        pub fn open(name: &str) -> Option<Self> {
            let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            unsafe {
                let h = CreateFileW(
                    name.as_ptr(),
                    0x40000000,
                    0,
                    ptr::null(),
                    3,
                    0,
                    ptr::null_mut(),
                );
                if h as isize == -1 {
                    return None;
                }
                let mode = 1u32; // PIPE_NOWAIT: bounded message writes never block tracking.
                if SetNamedPipeHandleState(h, &mode, ptr::null(), ptr::null()) == 0 {
                    CloseHandle(h);
                    return None;
                }
                Some(Self(h))
            }
        }
        pub fn write(&self, bytes: &[u8]) -> bool {
            let mut written = 0;
            unsafe {
                WriteFile(
                    self.0,
                    bytes.as_ptr(),
                    bytes.len() as u32,
                    &mut written,
                    ptr::null_mut(),
                ) != 0
                    && written as usize == bytes.len()
            }
        }
    }
    impl Drop for Pipe {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub fn fresh_session() -> Option<String> {
        let mut token = [0u8; 16];
        if unsafe { BCryptGenRandom(ptr::null_mut(), token.as_mut_ptr(), 16, 2) } < 0 {
            return None;
        }
        Some(token.iter().map(|b| format!("{b:02x}")).collect())
    }
}
#[cfg(not(windows))]
mod platform {
    pub struct Pipe;
    impl Pipe {
        pub fn open(_: &str) -> Option<Self> {
            None
        }
        pub fn write(&self, _: &[u8]) -> bool {
            false
        }
    }
    pub fn fresh_session() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alvr_common::{
        Pose,
        hmd_authority::{HmdAuthoritySessionEpoch, HmdRawValidity},
    };
    #[test]
    fn bounded_raw_sampling_preserves_definition_and_loss_boundaries() {
        let mut export = AuthorityExport::test_pipe(r"\\.\pipe\software-only".into());
        export.min_interval = Duration::from_millis(50);
        let e = HmdAuthorityEvidenceV1 {
            session_epoch: HmdAuthoritySessionEpoch([7; 16]),
            observation_id: 0,
            reference_space_generation: 0,
            source_locate_time: 0,
            source_pose: Pose::IDENTITY,
            validity: HmdRawValidity::default(),
        };
        export.last_sample = Some((e, Duration::from_secs(1)));
        let mut next = e;
        next.observation_id = 1;
        next.source_locate_time = 1;
        assert!(!export.sample_due(next, Duration::from_millis(1049)));
        assert!(export.sample_due(next, Duration::from_millis(1050)));
        next.reference_space_generation = 1;
        assert!(export.sample_due(next, Duration::from_millis(1001)));
        next = e;
        next.session_epoch = HmdAuthoritySessionEpoch([8; 16]);
        assert!(export.sample_due(next, Duration::from_millis(1001)));
        next = e;
        next.validity.position_valid = true;
        assert!(export.sample_due(next, Duration::from_millis(1001)));
        assert!(!export.sample_due(e, Duration::from_millis(999)));
        assert!(e.immutable_eq(&export.last_sample.unwrap().0));
    }
    #[test]
    fn export_exact_original_bits_and_signed_time() {
        let mut pose = Pose::IDENTITY;
        pose.position.x = -0.0;
        pose.position.y = 1.25;
        let e = HmdAuthorityEvidenceV1 {
            session_epoch: HmdAuthoritySessionEpoch([7; 16]),
            observation_id: i64::MAX as u64,
            reference_space_generation: 3,
            source_locate_time: -9,
            source_pose: pose,
            validity: HmdRawValidity::default(),
        };
        let frame = sample_frame(e);
        assert_eq!(frame["source_pose_bits"][0], 0x80000000u32);
        assert_eq!(frame["source_pose_bits"][1], 1.25f32.to_bits());
        assert_eq!(frame["source_locate_time_ns"], "-9");
        assert_eq!(frame["observation_id"], i64::MAX.to_string());
        assert_eq!(frame["source_token"], "07070707070707070707070707070707");
        assert_eq!(frame["validity_mask"], 0);
        if let Ok(path) = std::env::var("MONAKA_ALVR_EXPORT_FIXTURE") {
            std::fs::write(path, frame.to_string()).unwrap();
        }
    }
}
