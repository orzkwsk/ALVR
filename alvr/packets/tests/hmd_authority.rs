use alvr_common::{
    DeviceMotion, HEAD_ID, Pose,
    glam::{Quat, Vec3},
    hmd_authority::*,
};
use alvr_packets::{FaceData, TrackingData};
use std::time::Duration;

fn evidence(pose: Pose) -> HmdAuthorityEvidenceV1 {
    HmdAuthorityEvidenceV1 {
        session_epoch: HmdAuthoritySessionEpoch([0xa5; 16]),
        observation_id: 7,
        reference_space_generation: 3,
        source_locate_time: 99,
        source_pose: pose,
        validity: HmdRawValidity {
            position_valid: true,
            orientation_valid: true,
            position_tracked: false,
            orientation_tracked: true,
            view_position_valid: true,
            view_orientation_valid: true,
        },
    }
}
fn packet(pose: Pose) -> TrackingData {
    TrackingData {
        poll_timestamp: Duration::from_nanos(110),
        hmd_authority: Some(evidence(pose)),
        device_motions: vec![(
            *HEAD_ID,
            DeviceMotion {
                pose,
                ..DeviceMotion::IDENTITY
            },
        )],
        hand_skeletons: [None; 2],
        face: FaceData::default(),
        body: None,
    }
}
#[test]
fn packet_roundtrip_numeric_payloads_and_overhead() {
    // Same bincode 2 standard configuration as the live unreliable tracking stream.
    for bits in [
        0, 0x80000000, 1, 0x007fffff, 0x00800000, 0x7f7fffff, 0xff7fffff, 0x7fc01234,
    ] {
        let f = f32::from_bits(bits);
        let pose = Pose {
            position: Vec3::new(f, -0.0, f32::from_bits(1)),
            orientation: Quat::from_xyzw(f, 0.0, -0.0, 1.0),
        };
        let data = packet(pose);
        let encoded = bincode::serde::encode_to_vec(&data, bincode::config::standard()).unwrap();
        let (mut decoded, used): (TrackingData, usize) =
            bincode::serde::decode_from_slice(&encoded, bincode::config::standard()).unwrap();
        assert_eq!(used, encoded.len());
        assert!(decoded.validate_hmd_authority());
        assert!(
            decoded
                .hmd_authority
                .unwrap()
                .immutable_eq(&data.hmd_authority.unwrap())
        );
        assert_eq!(decoded.poll_timestamp, Duration::from_nanos(110));
        assert_eq!(decoded.hmd_authority.unwrap().source_locate_time, 99);
    }
    let mut data = packet(Pose::IDENTITY);
    // The pinned pre-prototype TrackingData is the ordered tuple of these five
    // fields. Bincode struct/tuple field encoding is identical, with no names.
    let upstream = bincode::serde::encode_to_vec(
        (
            data.poll_timestamp,
            &data.device_motions,
            data.hand_skeletons,
            &data.face,
            &data.body,
        ),
        bincode::config::standard(),
    )
    .unwrap()
    .len();
    let added = bincode::serde::encode_to_vec(&data, bincode::config::standard())
        .unwrap()
        .len();
    data.hmd_authority = None;
    let absent = bincode::serde::encode_to_vec(&data, bincode::config::standard())
        .unwrap()
        .len();
    println!(
        "PROTOTYPE overhead: Some={} None={} added={} bytes/sample",
        added,
        absent,
        added - absent
    );
    println!(
        "UPSTREAM comparison: original={} patched-None={} patched-Some={} added-to-original={} bytes/sample",
        upstream,
        absent,
        added,
        added - upstream
    );
    // Counter/time varints grow over the session: measure the signed-domain upper case too.
    let mut e = evidence(Pose::IDENTITY);
    e.observation_id = AUTHORITY_MAX;
    e.reference_space_generation = AUTHORITY_MAX;
    e.source_locate_time = i64::MAX;
    data.hmd_authority = Some(e);
    let upper = bincode::serde::encode_to_vec(&data, bincode::config::standard())
        .unwrap()
        .len();
    println!(
        "PROTOTYPE max-varint overhead: Some={} added={} bytes/sample",
        upper,
        upper - absent
    );
    println!(
        "UPSTREAM max-varint comparison: added-to-original={} bytes/sample",
        upper - upstream
    );
}
#[test]
fn malformed_evidence_discards_only_authority() {
    let mut data = packet(Pose::IDENTITY);
    data.hmd_authority.as_mut().unwrap().source_pose.position.x = -0.0;
    let motion = pose_bits(data.device_motions[0].1.pose);
    assert!(!data.validate_hmd_authority());
    assert!(data.hmd_authority.is_none());
    assert_eq!(pose_bits(data.device_motions[0].1.pose), motion);
    assert_eq!(data.poll_timestamp, Duration::from_nanos(110));
    let mut data = packet(Pose::IDENTITY);
    data.device_motions.clear();
    assert!(!data.validate_hmd_authority());
    let mut data = packet(Pose::IDENTITY);
    data.device_motions.push(data.device_motions[0]);
    assert!(!data.validate_hmd_authority());
    assert_eq!(data.device_motions.len(), 2);
    data.hmd_authority = None;
    assert!(data.validate_hmd_authority());
}
