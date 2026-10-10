use super::*;
use alvr_common::hmd_authority::*;
use alvr_session::SessionConfig;

#[cfg(windows)]
#[test]
fn native_pipe_exports_only_accepted_pretransform_and_fences_reconnect() {
    use crate::authority_export::AuthorityExport;
    use std::{ffi::c_void, ptr};
    type Handle = *mut c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            access: u32,
            mode: u32,
            count: u32,
            out: u32,
            input: u32,
            timeout: u32,
            sa: *const c_void,
        ) -> Handle;
        fn ConnectNamedPipe(h: Handle, overlap: *mut c_void) -> i32;
        fn DisconnectNamedPipe(h: Handle) -> i32;
        fn ReadFile(
            h: Handle,
            bytes: *mut u8,
            len: u32,
            read: *mut u32,
            overlap: *mut c_void,
        ) -> i32;
        fn CloseHandle(h: Handle) -> i32;
    }
    struct Server(Handle);
    impl Server {
        fn read(&self) -> serde_json::Value {
            let mut b = [0u8; 2048];
            let mut n = 0;
            unsafe {
                ConnectNamedPipe(self.0, ptr::null_mut());
                assert_ne!(
                    ReadFile(self.0, b.as_mut_ptr(), 2048, &mut n, ptr::null_mut()),
                    0
                );
            }
            assert!(n > 0);
            serde_json::from_slice(&b[..n as usize]).unwrap()
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let name = format!(r"\\.\pipe\monaka-software-test-{}", std::process::id());
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let server = Server(unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            1 | 0x80000,
            4 | 2 | 1 | 8,
            1,
            8192,
            8192,
            0,
            ptr::null(),
        )
    });
    assert_ne!(server.0 as isize, -1);
    unsafe {
        ConnectNamedPipe(server.0, ptr::null_mut());
    }
    let mut manager = TrackingManager::new(4);
    manager.authority_export = Some(AuthorityExport::test_pipe(name));
    let mut e = evidence(
        0,
        Pose {
            position: Vec3::new(-0.0, 1.25, -3.0),
            ..Pose::IDENTITY
        },
    );
    e.validity = HmdRawValidity {
        position_valid: true,
        orientation_valid: true,
        position_tracked: true,
        orientation_tracked: true,
        view_position_valid: true,
        view_orientation_valid: true,
    };
    manager.set_recenter_mapping(Pose {
        position: Vec3::X,
        ..Pose::IDENTITY
    });
    feed(&mut manager, 100, Some(e));
    let connected = server.read();
    let sample = server.read();
    assert_eq!(connected["event"], "connected");
    assert_eq!(sample["event"], "sample");
    assert_eq!(
        sample["source_pose_bits"],
        serde_json::json!(pose_bits(e.source_pose))
    );
    assert_eq!(sample["validity_mask"], 63);
    assert_eq!(sample["export_session"], connected["export_session"]);
    assert_ne!(
        pose_bits(
            manager
                .get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(100))
                .unwrap()
                .motion
                .pose
        ),
        pose_bits(e.source_pose)
    );
    feed(&mut manager, 101, Some(e));
    assert_eq!(manager.authority_export.as_ref().unwrap().rejected, 1);
    manager.reset_history(4);
    let disconnected = server.read();
    assert_eq!(disconnected["event"], "disconnect");
    unsafe {
        assert_ne!(DisconnectNamedPipe(server.0), 0);
        ConnectNamedPipe(server.0, ptr::null_mut());
    }
    e.observation_id = 1;
    e.source_locate_time = 1;
    feed(&mut manager, 102, Some(e));
    let next_connected = server.read();
    let next_sample = server.read();
    assert_ne!(
        next_connected["export_session"],
        connected["export_session"]
    );
    assert_eq!(next_sample["source_token"], sample["source_token"]);
    if let Ok(path) = std::env::var("MONAKA_ALVR_NATIVE_TEST_FRAMES") {
        let rows = [connected, sample, disconnected, next_connected, next_sample];
        std::fs::write(
            path,
            rows.iter()
                .map(|j| j.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
    }
    // No consumer reads now. Nonblocking bounded writes must close and count drops.
    for i in 2..256 {
        e.observation_id = i;
        e.source_locate_time = i as i64;
        feed(&mut manager, 103 + i, Some(e));
    }
    assert!(manager.authority_export.as_ref().unwrap().dropped > 0);
}

fn evidence(obs: u64, pose: Pose) -> HmdAuthorityEvidenceV1 {
    HmdAuthorityEvidenceV1 {
        session_epoch: HmdAuthoritySessionEpoch([7; 16]),
        observation_id: obs,
        reference_space_generation: 0,
        source_locate_time: obs as i64,
        source_pose: pose,
        validity: HmdRawValidity::default(),
    }
}
fn feed(manager: &mut TrackingManager, ts: u64, e: Option<HmdAuthorityEvidenceV1>) {
    let pose = e.map(|e| e.source_pose).unwrap_or(Pose::IDENTITY);
    manager.report_device_motions(
        &SessionConfig::default().to_settings().headset,
        Duration::from_nanos(ts),
        &[(
            *inp::HEAD_ID,
            DeviceMotion {
                pose,
                linear_velocity: Vec3::X,
                ..DeviceMotion::IDENTITY
            },
        )],
        e,
    );
}
#[test]
fn exact_and_nearest_newer_keep_actual_identity_and_timestamp() {
    let mut m = TrackingManager::new(5);
    let a = evidence(0, Pose::IDENTITY);
    let b = evidence(
        1,
        Pose {
            position: Vec3::new(2.0, 3.0, 4.0),
            ..Pose::IDENTITY
        },
    );
    feed(&mut m, 100, Some(a));
    feed(&mut m, 110, Some(b));
    let exact = m
        .get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(100))
        .unwrap();
    assert!(exact.hmd_authority.unwrap().immutable_eq(&a));
    let selected = m
        .get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(105))
        .unwrap();
    assert_eq!(selected.tracking_timestamp, Duration::from_nanos(110));
    assert!(selected.hmd_authority.unwrap().immutable_eq(&b));
    assert_eq!(pose_bits(selected.motion.pose), pose_bits(b.source_pose));
    assert!(
        m.get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(111))
            .is_none()
    );
    assert_eq!(
        pose_bits(
            m.get_device_motion(*inp::HEAD_ID, Duration::from_nanos(105))
                .unwrap()
                .pose
        ),
        pose_bits(selected.motion.pose)
    );
}
#[test]
fn recenter_snapshots_and_prediction_do_not_mutate_source() {
    let mut m = TrackingManager::new(5);
    let a = Pose::IDENTITY;
    let b = Pose {
        position: Vec3::new(1.0, 2.0, 3.0),
        ..Pose::IDENTITY
    };
    m.set_recenter_mapping(a);
    assert_eq!(m.server_recenter_epoch, 0);
    m.set_recenter_mapping(b);
    assert_eq!(m.server_recenter_epoch, 1);
    m.set_recenter_mapping(b);
    assert_eq!(m.server_recenter_epoch, 1);
    let e = evidence(0, Pose::IDENTITY);
    feed(&mut m, 100, Some(e));
    let selected = m
        .get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(99))
        .unwrap();
    assert_eq!(selected.server_recenter_epoch, 1);
    assert_eq!(pose_bits(selected.inverse_recentering_origin), pose_bits(b));
    assert!(selected.hmd_authority.unwrap().immutable_eq(&e));
    assert_ne!(pose_bits(selected.motion.pose), pose_bits(e.source_pose));
    let predicted = selected
        .motion
        .predict(Duration::ZERO, Duration::from_secs(1));
    assert_ne!(pose_bits(predicted.pose), pose_bits(selected.motion.pose));
    assert!(selected.hmd_authority.unwrap().immutable_eq(&e));
    m.set_recenter_mapping(a);
    assert_eq!(m.server_recenter_epoch, 2);
    let old = m
        .get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(100))
        .unwrap();
    assert_eq!(old.server_recenter_epoch, 1);
    assert_eq!(pose_bits(old.inverse_recentering_origin), pose_bits(b));
}
#[test]
fn replay_generic_fallback_conflict_revocation_and_reconnect_high_water() {
    let mut m = TrackingManager::new(10);
    let e = evidence(5, Pose::IDENTITY);
    feed(&mut m, 100, Some(e));
    feed(&mut m, 110, Some(e));
    assert!(
        m.get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(110))
            .unwrap()
            .hmd_authority
            .is_none()
    );
    assert!(
        m.get_device_motion(*inp::HEAD_ID, Duration::from_nanos(110))
            .is_some()
    );
    m.reset_history(10);
    let lower = evidence(4, Pose::IDENTITY);
    feed(&mut m, 120, Some(lower));
    assert!(
        m.get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(120))
            .unwrap()
            .hmd_authority
            .is_none()
    );
    let newer = evidence(6, Pose::IDENTITY);
    feed(&mut m, 130, Some(newer));
    let mut conflict = newer;
    conflict.validity.position_valid = true;
    feed(&mut m, 140, Some(conflict));
    assert!(
        m.get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(130))
            .unwrap()
            .hmd_authority
            .is_none()
    );
    assert!(
        m.get_device_motion(*inp::HEAD_ID, Duration::from_nanos(140))
            .is_some()
    );
    let mut fresh = evidence(0, Pose::IDENTITY);
    fresh.session_epoch = HmdAuthoritySessionEpoch([8; 16]);
    feed(&mut m, 150, Some(fresh));
    assert!(
        m.get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(150))
            .unwrap()
            .hmd_authority
            .is_some()
    );
    feed(&mut m, 160, None);
    assert!(
        m.get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(160))
            .unwrap()
            .hmd_authority
            .is_none()
    );
}
#[test]
fn malformed_binding_and_recenter_epoch_overflow_fail_authority_only() {
    let mut m = TrackingManager::new(10);
    let e = evidence(
        0,
        Pose {
            position: Vec3::X,
            ..Pose::IDENTITY
        },
    );
    let config = SessionConfig::default().to_settings().headset;
    m.report_device_motions(
        &config,
        Duration::ZERO,
        &[(*inp::HEAD_ID, DeviceMotion::IDENTITY)],
        Some(e),
    );
    assert!(
        m.get_device_motion_sample(*inp::HEAD_ID, Duration::ZERO)
            .unwrap()
            .hmd_authority
            .is_none()
    );
    m.report_device_motions(
        &config,
        Duration::from_nanos(1),
        &[(*inp::HEAD_ID, DeviceMotion::IDENTITY); 2],
        Some(evidence(0, Pose::IDENTITY)),
    );
    assert!(
        m.get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(1))
            .unwrap()
            .hmd_authority
            .is_none()
    );
    m.server_recenter_epoch = u64::MAX;
    m.set_recenter_mapping(Pose {
        position: Vec3::X,
        ..Pose::IDENTITY
    });
    feed(&mut m, 3, Some(e));
    assert!(
        m.get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(3))
            .unwrap()
            .hmd_authority
            .is_none()
    );
}

#[test]
fn same_decoded_packet_through_history_and_prediction() {
    // Integration at the live codec/manager seam, using the packet's actual source values.
    let mut m = TrackingManager::new(10);
    let config = SessionConfig::default().to_settings().headset;
    let mut b = evidence(
        1,
        Pose {
            position: Vec3::new(2.0, 3.0, 4.0),
            ..Pose::IDENTITY
        },
    );
    b.source_locate_time = 99; // Deliberately distinct from poll and query timestamps.
    for (time, e) in [(100, evidence(0, Pose::IDENTITY)), (110, b)] {
        let packet = TrackingData {
            poll_timestamp: Duration::from_nanos(time),
            hmd_authority: Some(e),
            device_motions: vec![(
                *inp::HEAD_ID,
                DeviceMotion {
                    pose: e.source_pose,
                    linear_velocity: Vec3::X,
                    ..DeviceMotion::IDENTITY
                },
            )],
            hand_skeletons: [None; 2],
            face: alvr_packets::FaceData::default(),
            body: None,
        };
        let bytes = bincode::serde::encode_to_vec(&packet, bincode::config::standard()).unwrap();
        let (mut decoded, size): (TrackingData, usize) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(size, bytes.len());
        assert!(decoded.validate_hmd_authority());
        m.report_device_motions(
            &config,
            decoded.poll_timestamp,
            &decoded.device_motions,
            decoded.hmd_authority,
        );
    }
    let selected = m
        .get_device_motion_sample(*inp::HEAD_ID, Duration::from_nanos(105))
        .unwrap();
    assert_eq!(selected.tracking_timestamp, Duration::from_nanos(110));
    assert!(selected.hmd_authority.unwrap().immutable_eq(&b));
    assert_eq!(selected.hmd_authority.unwrap().source_locate_time, 99);
    let provider = selected
        .motion
        .predict(Duration::from_nanos(105), Duration::from_secs(1));
    assert_ne!(pose_bits(provider.pose), pose_bits(b.source_pose));
    assert!(selected.hmd_authority.unwrap().immutable_eq(&b));
}
