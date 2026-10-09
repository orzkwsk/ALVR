//! Isolated OpenXR authority prototype. No runtime, network, entropy or logger dependency.
//! Times are signed OpenXR locate evaluation nanoseconds, never sensor acquisition times.
use crate::Pose;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

pub const AUTHORITY_MAX: u64 = i64::MAX as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HmdAuthoritySessionEpoch(pub [u8; 16]);

pub fn pose_bits(pose: Pose) -> [u32; 7] {
    let p = pose.position.to_array();
    let q = pose.orientation.to_array();
    [
        p[0].to_bits(),
        p[1].to_bits(),
        p[2].to_bits(),
        q[0].to_bits(),
        q[1].to_bits(),
        q[2].to_bits(),
        q[3].to_bits(),
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HmdRawValidity {
    pub position_valid: bool,
    pub orientation_valid: bool,
    pub position_tracked: bool,
    pub orientation_tracked: bool,
    pub view_position_valid: bool,
    pub view_orientation_valid: bool,
}

impl HmdRawValidity {
    pub fn bits(self) -> u32 {
        u32::from(self.position_valid)
            | (u32::from(self.orientation_valid) << 1)
            | (u32::from(self.position_tracked) << 2)
            | (u32::from(self.orientation_tracked) << 3)
            | (u32::from(self.view_position_valid) << 4)
            | (u32::from(self.view_orientation_valid) << 5)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct HmdAuthorityEvidenceV1 {
    pub session_epoch: HmdAuthoritySessionEpoch,
    pub observation_id: u64,
    pub reference_space_generation: u64,
    pub source_locate_time: i64,
    pub source_pose: Pose,
    pub validity: HmdRawValidity,
}

impl HmdAuthorityEvidenceV1 {
    pub fn immutable_eq(&self, other: &Self) -> bool {
        self.session_epoch == other.session_epoch
            && self.observation_id == other.observation_id
            && self.reference_space_generation == other.reference_space_generation
            && self.source_locate_time == other.source_locate_time
            && pose_bits(self.source_pose) == pose_bits(other.source_pose)
            && self.validity == other.validity
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityLifecycle {
    Active,
    Invalid,
    Exhausted,
    Retired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceSpaceKind {
    Stage,
    View,
    Irrelevant,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
pub struct PendingReferenceSpaceChange {
    pub reference_space_type: ReferenceSpaceKind,
    pub change_time: i64,
    pub next_generation: u64,
    pub pose_valid: bool,
    pub pose_in_previous_space: Pose,
}

#[derive(Default)]
pub struct AuthoritySessionRegistry {
    issued: HashSet<HmdAuthoritySessionEpoch>,
}

impl AuthoritySessionRegistry {
    /// Caller supplies OS entropy in production, a deterministic factory in tests.
    pub fn start(&mut self, token: HmdAuthoritySessionEpoch) -> Option<HmdAuthorityState> {
        self.issued
            .insert(token)
            .then(|| HmdAuthorityState::new(token))
    }
}

pub struct HmdAuthorityState {
    pub lifecycle: AuthorityLifecycle,
    token: HmdAuthoritySessionEpoch,
    next_observation: u64,
    generation: u64,
    next_generation: u64,
    pending: VecDeque<PendingReferenceSpaceChange>,
    last_event_time: Option<i64>,
    pub last_authoritative_sample_time: Option<i64>,
    begun: bool,
    emitting: bool,
}

impl HmdAuthorityState {
    /// Entropy failure is authority-only; this sentinel can never emit an epoch.
    pub fn unavailable() -> Self {
        let mut state = Self::new(HmdAuthoritySessionEpoch([0; 16]));
        state.invalidate();
        state
    }
    fn new(token: HmdAuthoritySessionEpoch) -> Self {
        Self {
            lifecycle: AuthorityLifecycle::Active,
            token,
            next_observation: 0,
            generation: 0,
            next_generation: 0,
            pending: VecDeque::new(),
            last_event_time: None,
            last_authoritative_sample_time: None,
            begun: false,
            emitting: false,
        }
    }
    pub fn begin(&mut self) {
        if self.begun {
            self.invalidate();
        }
        self.begun = true;
        self.emitting = self.lifecycle == AuthorityLifecycle::Active;
    }
    /// STOPPING cannot establish a fresh baseline on the same XrSession.
    pub fn stop(&mut self) {
        self.emitting = false;
        self.invalidate();
    }
    pub fn invalidate(&mut self) {
        if self.lifecycle != AuthorityLifecycle::Retired {
            self.lifecycle = AuthorityLifecycle::Invalid;
        }
        self.emitting = false;
    }
    pub fn events_lost(&mut self, _lost_event_count: u32) {
        // Count cannot reconstruct which relevant reference events were lost.
        self.invalidate();
    }
    pub fn pending_generation(&self) -> u64 {
        self.next_generation
    }
    pub fn retire(&mut self) {
        self.lifecycle = AuthorityLifecycle::Retired;
        self.emitting = false;
    }
    pub fn reference_change(
        &mut self,
        kind: ReferenceSpaceKind,
        time: i64,
        pose_valid: bool,
        previous: Pose,
    ) {
        if self.lifecycle != AuthorityLifecycle::Active || kind == ReferenceSpaceKind::Irrelevant {
            return;
        }
        if kind == ReferenceSpaceKind::Unknown
            || self.last_event_time.is_some_and(|t| time <= t)
            || self
                .last_authoritative_sample_time
                .is_some_and(|t| t >= time)
            || self.next_generation == AUTHORITY_MAX
            || self.pending.len() >= 1024
        {
            self.invalidate();
            return;
        }
        self.next_generation += 1;
        self.last_event_time = Some(time);
        self.pending.push_back(PendingReferenceSpaceChange {
            reference_space_type: kind,
            change_time: time,
            next_generation: self.next_generation,
            pose_valid,
            pose_in_previous_space: previous,
        });
    }
    /// Call once after successful acquisition, while holding the event/sample gate through packet send.
    /// Failed acquisition never calls this method. No OpenXR call may hold the gate.
    pub fn sample(
        &mut self,
        time: i64,
        pose: Pose,
        validity: HmdRawValidity,
    ) -> Option<HmdAuthorityEvidenceV1> {
        if self.lifecycle != AuthorityLifecycle::Active || !self.emitting {
            return None;
        }
        if self
            .last_authoritative_sample_time
            .is_some_and(|t| time < t)
        {
            self.invalidate();
            return None;
        }
        while self
            .pending
            .front()
            .is_some_and(|event| time >= event.change_time)
        {
            self.generation = self.pending.pop_front().unwrap().next_generation;
        }
        let result = HmdAuthorityEvidenceV1 {
            session_epoch: self.token,
            observation_id: self.next_observation,
            reference_space_generation: self.generation,
            source_locate_time: time,
            source_pose: pose,
            validity,
        };
        self.last_authoritative_sample_time = Some(time);
        if self.next_observation == AUTHORITY_MAX {
            self.lifecycle = AuthorityLifecycle::Exhausted;
            self.emitting = false;
        } else {
            self.next_observation += 1;
        }
        Some(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityReceiveResult {
    Accepted,
    Duplicate,
    Rejected,
    Retired,
}

#[derive(Default)]
pub struct AuthorityReceiver {
    active: Option<HmdAuthoritySessionEpoch>,
    sessions: HashMap<HmdAuthoritySessionEpoch, (HmdAuthorityEvidenceV1, bool)>,
}

impl AuthorityReceiver {
    pub fn eligible(&self, token: HmdAuthoritySessionEpoch) -> bool {
        self.active == Some(token)
            && self
                .sessions
                .get(&token)
                .is_some_and(|(_, retired)| !retired)
    }
    pub fn receive(&mut self, evidence: HmdAuthorityEvidenceV1) -> AuthorityReceiveResult {
        use AuthorityReceiveResult::*;
        if evidence.observation_id > AUTHORITY_MAX
            || evidence.reference_space_generation > AUTHORITY_MAX
        {
            return Rejected;
        }
        let token = evidence.session_epoch;
        if let Some((previous, retired)) = self.sessions.get_mut(&token) {
            if *retired || self.active != Some(token) {
                return Rejected;
            }
            if evidence.observation_id < previous.observation_id {
                return Rejected;
            }
            if evidence.observation_id == previous.observation_id {
                if previous.immutable_eq(&evidence) {
                    return Duplicate;
                }
                *retired = true;
                return Retired;
            }
            if evidence.reference_space_generation < previous.reference_space_generation {
                return Rejected;
            }
            *previous = evidence;
        } else {
            if let Some(active) = self.active
                && let Some((_, retired)) = self.sessions.get_mut(&active)
            {
                *retired = true;
            }
            self.active = Some(token);
            self.sessions.insert(token, (evidence, false));
        }
        Accepted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> HmdAuthorityState {
        let mut state = AuthoritySessionRegistry::default()
            .start(HmdAuthoritySessionEpoch([7; 16]))
            .unwrap();
        state.begin();
        state
    }
    fn sample(state: &mut HmdAuthorityState, time: i64) -> Option<HmdAuthorityEvidenceV1> {
        state.sample(time, Pose::IDENTITY, HmdRawValidity::default())
    }
    #[test]
    fn fresh_tokens_and_observation_exhaustion() {
        let mut registry = AuthoritySessionRegistry::default();
        let token = HmdAuthoritySessionEpoch([1; 16]);
        let mut s = registry.start(token).unwrap();
        assert!(registry.start(token).is_none());
        assert!(registry.start(HmdAuthoritySessionEpoch([2; 16])).is_some());
        assert!(sample(&mut s, 0).is_none());
        s.begin();
        // A failed acquisition does not call sample, so does not consume an ID.
        for id in 0..3 {
            let e = sample(&mut s, id as i64).unwrap();
            assert_eq!(e.observation_id, id);
            assert_eq!(e.reference_space_generation, 0);
        }
        s.next_observation = AUTHORITY_MAX;
        assert_eq!(sample(&mut s, 4).unwrap().observation_id, AUTHORITY_MAX);
        assert_eq!(s.lifecycle, AuthorityLifecycle::Exhausted);
        assert!(sample(&mut s, 5).is_none());
    }
    #[test]
    fn exact_boundary_multiple_events_and_false_pose_valid() {
        let mut s = state();
        s.reference_change(ReferenceSpaceKind::Stage, 100, false, Pose::IDENTITY);
        s.reference_change(ReferenceSpaceKind::View, 200, true, Pose::IDENTITY);
        for (t, g) in [(99, 0), (100, 1), (101, 1), (199, 1), (200, 2), (201, 2)] {
            assert_eq!(sample(&mut s, t).unwrap().reference_space_generation, g);
        }
    }
    #[test]
    fn late_ambiguous_lost_stopped_and_retired_fail_closed() {
        for time in [99, 100] {
            let mut s = state();
            sample(&mut s, 100).unwrap();
            s.reference_change(ReferenceSpaceKind::Stage, time, true, Pose::IDENTITY);
            assert!(sample(&mut s, 101).is_none());
        }
        for time in [99, 100] {
            let mut s = state();
            s.reference_change(ReferenceSpaceKind::Stage, 100, true, Pose::IDENTITY);
            s.reference_change(ReferenceSpaceKind::View, time, true, Pose::IDENTITY);
            assert!(sample(&mut s, 101).is_none());
        }
        let mut s = state();
        s.invalidate();
        s.begin();
        assert!(sample(&mut s, 1).is_none());
        let mut s = state();
        s.stop();
        s.begin();
        assert!(sample(&mut s, 1).is_none());
        let mut s = state();
        s.retire();
        s.begin();
        assert_eq!(s.lifecycle, AuthorityLifecycle::Retired);
        assert!(sample(&mut s, 1).is_none());
        let mut s = state();
        s.next_generation = AUTHORITY_MAX;
        s.reference_change(ReferenceSpaceKind::Stage, 100, true, Pose::IDENTITY);
        assert!(sample(&mut s, 100).is_none());
    }
    #[test]
    fn space_policy_and_time_regression() {
        let mut s = state();
        s.reference_change(ReferenceSpaceKind::Irrelevant, 1, false, Pose::IDENTITY);
        assert_eq!(sample(&mut s, 2).unwrap().reference_space_generation, 0);
        assert!(sample(&mut s, 1).is_none());
        let mut s = state();
        s.reference_change(ReferenceSpaceKind::Unknown, 1, false, Pose::IDENTITY);
        assert!(sample(&mut s, 2).is_none());
    }

    #[test]
    fn failed_acquisition_and_event_loss_do_not_create_authority() {
        let mut s = state();
        let failed: Option<(i64, Pose, HmdRawValidity)> = None;
        assert!(
            failed
                .and_then(|(time, pose, flags)| s.sample(time, pose, flags))
                .is_none()
        );
        assert_eq!(sample(&mut s, 1).unwrap().observation_id, 0);
        for count in [0, 1, u32::MAX] {
            let mut s = state();
            s.events_lost(count);
            s.begin();
            assert_eq!(s.lifecycle, AuthorityLifecycle::Invalid);
            assert!(sample(&mut s, 1).is_none());
        }
        let mut s = HmdAuthorityState::unavailable();
        s.begin();
        assert!(sample(&mut s, 1).is_none());
    }

    #[test]
    fn last_generation_and_numeric_replay_are_exact() {
        let mut s = state();
        s.next_generation = AUTHORITY_MAX - 1;
        s.reference_change(ReferenceSpaceKind::View, 100, false, Pose::IDENTITY);
        let mut e = sample(&mut s, 100).unwrap();
        assert_eq!(e.reference_space_generation, AUTHORITY_MAX);
        s.reference_change(ReferenceSpaceKind::Stage, 101, true, Pose::IDENTITY);
        assert!(sample(&mut s, 101).is_none());
        e.source_pose.position.x = f32::from_bits(0x7fc01234);
        let mut receiver = AuthorityReceiver::default();
        assert_eq!(receiver.receive(e), AuthorityReceiveResult::Accepted);
        assert_eq!(receiver.receive(e), AuthorityReceiveResult::Duplicate);
        e.source_pose.position.x = f32::from_bits(0x7fc01235);
        assert_eq!(receiver.receive(e), AuthorityReceiveResult::Retired);
        let mut receiver = AuthorityReceiver::default();
        e.observation_id = AUTHORITY_MAX + 1;
        assert_eq!(receiver.receive(e), AuthorityReceiveResult::Rejected);
    }
    #[test]
    fn replay_conflict_session_switch_and_generation_decrease() {
        use AuthorityReceiveResult::*;
        let mut r = AuthorityReceiver::default();
        let mut s = state();
        let mut e = sample(&mut s, 1).unwrap();
        assert_eq!(r.receive(e), Accepted);
        assert_eq!(r.receive(e), Duplicate);
        e.observation_id = 2;
        e.reference_space_generation = 1;
        assert_eq!(r.receive(e), Accepted);
        let mut lower = e;
        lower.observation_id = 1;
        assert_eq!(r.receive(lower), Rejected);
        let mut decreasing = e;
        decreasing.observation_id = 3;
        decreasing.reference_space_generation = 0;
        assert_eq!(r.receive(decreasing), Rejected);
        let mut conflict = e;
        conflict.source_pose.position.x = -0.0;
        assert_eq!(r.receive(conflict), Retired);
        assert!(!r.eligible(e.session_epoch));
        assert_eq!(r.receive(e), Rejected);
        e.session_epoch = HmdAuthoritySessionEpoch([8; 16]);
        e.observation_id = 0;
        assert_eq!(r.receive(e), Accepted);
        e.session_epoch = HmdAuthoritySessionEpoch([9; 16]);
        assert_eq!(r.receive(e), Accepted);
        e.session_epoch = HmdAuthoritySessionEpoch([8; 16]);
        assert_eq!(r.receive(e), Rejected);
    }
}
