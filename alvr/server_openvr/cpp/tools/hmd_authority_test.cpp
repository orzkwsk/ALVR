// Boundary harness: the runner appends the unmodified production function bodies.
// Only unrelated driver construction, controllers/video and the SteamVR host are fakes.
#include "alvr_server/HmdAuthority.h"
#include <cassert>
#include <fstream>
#include <memory>
#include <vector>

struct FakeHost : vr::IVRServerDriverHost {
    unsigned count = 0;
    vr::DriverPose_t output { };
    bool TrackedDeviceAdded(
        const char*, vr::ETrackedDeviceClass, vr::ITrackedDeviceServerDriver*
    ) override {
        return false;
    }
    void
    TrackedDevicePoseUpdated(uint32_t id, const vr::DriverPose_t& pose, uint32_t size) override {
        assert(id == 42 && size == sizeof(vr::DriverPose_t));
        ++count;
        output = pose;
    }
    void VsyncEvent(double) override { }
    void
    VendorSpecificEvent(uint32_t, vr::EVREventType, const vr::VREvent_Data_t&, double) override { }
    bool IsExiting() override { return false; }
    bool PollNextEvent(vr::VREvent_t*, uint32_t) override { return false; }
    void GetRawTrackedDevicePoses(float, vr::TrackedDevicePose_t*, uint32_t) override { }
    void RequestRestart(const char*, const char*, const char*, const char*) override {
        assert(false);
    }
    uint32_t GetFrameTimings(vr::Compositor_FrameTiming*, uint32_t) override { return 0; }
    void
    SetDisplayEyeToHead(uint32_t, const vr::HmdMatrix34_t&, const vr::HmdMatrix34_t&) override { }
    void SetDisplayProjectionRaw(uint32_t, const vr::HmdRect2_t&, const vr::HmdRect2_t&) override {
    }
    void SetRecommendedRenderTargetSize(uint32_t, uint32_t, uint32_t) override { }
} host;
struct FakeContext : vr::IVRDriverContext {
    void* GetGenericInterface(const char* name, vr::EVRInitError* error) override {
        assert(std::strcmp(name, vr::IVRServerDriverHost_Version) == 0);
        if (error)
            *error = vr::VRInitError_None;
        return &host;
    }
    vr::DriverHandle_t GetDriverHandle() override { return 0; }
} context;

void Debug(const char*) { }
vr::HmdQuaternion_t HmdQuaternion_Init(double w, double x, double y, double z) {
    return { w, x, y, z };
}
struct TrackedDevice {
    uint32_t object_id = 42;
    vr::DriverPose_t last_pose { };
    void submit_pose(vr::DriverPose_t);
};
struct PoseHistory {
    unsigned count = 0;
    void OnPoseUpdated(uint64_t, FfiDeviceMotion) { ++count; }
};
struct ViveTrackerProxy {
    void update() { }
};
struct Hmd : TrackedDevice {
    std::shared_ptr<PoseHistory> m_poseHistory = std::make_shared<PoseHistory>();
    std::shared_ptr<ViveTrackerProxy> m_viveTrackerProxy;
    HmdAuthorityAssociationV1 last_hmd_authority;
    HmdAuthoritySubmitGate hmd_authority_submit_gate;
    void OnPoseUpdated(uint64_t, FfiDeviceMotion, FfiHmdAuthorityEvidenceV1);
};
struct Hand {
    void OnPoseUpdate(uint64_t, float, FfiHandData) { }
};
struct FakeViveTracker {
    void OnPoseUpdated(uint64_t, const FfiDeviceMotion*) { }
};
struct Provider {
    Hmd* hmd = nullptr;
    Hand* left_hand_tracker = nullptr;
    Hand* right_hand_tracker = nullptr;
    Hand* left_controller = nullptr;
    Hand* right_controller = nullptr;
    std::map<uint64_t, FakeViveTracker*> tracked_devices;
} g_driver_provider;
const std::vector<uint64_t> BODY_IDS;
Settings settings { };
const Settings* Settings_Instance() { return &settings; }

uint64_t double_bits(double value) {
    uint64_t bits;
    std::memcpy(&bits, &value, 8);
    return bits;
}
void check_output(const Hmd& hmd, FfiDeviceMotion motion) {
    const auto& p = host.output;
    assert(
        double_bits(p.qRotation.x) == double_bits(static_cast<double>(motion.pose.orientation.x))
    );
    assert(
        double_bits(p.qRotation.y) == double_bits(static_cast<double>(motion.pose.orientation.y))
    );
    assert(
        double_bits(p.qRotation.z) == double_bits(static_cast<double>(motion.pose.orientation.z))
    );
    assert(
        double_bits(p.qRotation.w) == double_bits(static_cast<double>(motion.pose.orientation.w))
    );
    for (unsigned i = 0; i < 3; ++i)
        assert(
            double_bits(p.vecPosition[i])
            == double_bits(static_cast<double>(motion.pose.position[i]))
        );
    assert(std::memcmp(&p, &hmd.last_hmd_authority.providerOutput, sizeof(p)) == 0);
    assert(p.poseIsValid && p.deviceIsConnected && p.result == vr::TrackingResult_Running_OK);
}
int main(int argc, char** argv) {
    vr::VRDriverContext() = &context;
    Hmd hmd;
    g_driver_provider.hmd = &hmd;
    FfiDeviceMotion motion { };
    motion.pose = { { 0.0f, -0.0f, 0.25f, 0.75f }, { 2.0f, -0.0f, 4.0f } };
    FfiHmdAuthorityEvidenceV1 evidence { };
    evidence.present = 1;
    evidence.observationId = 7;
    evidence.referenceSpaceGeneration = 3;
    evidence.sourceLocateTime = 99;
    evidence.selectedTrackingTimestamp = 110;
    evidence.predictionFromTimestamp = 105;
    evidence.predictionTargetTimestamp = 120;
    evidence.serverRecenterEpoch = 9;
    evidence.rawValidityBits = 36;
    for (unsigned i = 0; i < 16; ++i)
        evidence.sessionEpoch[i] = static_cast<uint8_t>(i);
    evidence.sourcePose = { { 0, 0, 0, 1 }, { -0.0f, 1.0f, 3.0f } };
    SetTracking(105, 0, motion, evidence, { }, { }, nullptr, 0);
    assert(host.count == 1 && hmd.m_poseHistory->count == 1);
    assert(hmd.last_hmd_authority.evidence.present == 1);
    assert(std::memcmp(&evidence, &hmd.last_hmd_authority.evidence, sizeof(evidence)) == 0);
    check_output(hmd, motion);
    // A selected source may be requested twice: authority associates only the first output.
    SetTracking(106, 0, motion, evidence, { }, { }, nullptr, 0);
    assert(host.count == 2 && hmd.last_hmd_authority.evidence.present == 0);
    check_output(hmd, motion);
    auto conflict = evidence;
    conflict.sourcePose.position[0] = 0.0f;
    SetTracking(107, 0, motion, conflict, { }, { }, nullptr, 0);
    assert(host.count == 3 && hmd.last_hmd_authority.evidence.present == 0);
    evidence.observationId = 8;
    SetTracking(108, 0, motion, evidence, { }, { }, nullptr, 0);
    assert(host.count == 4 && hmd.last_hmd_authority.evidence.present == 0);
    // Fresh token accepts a fresh high-water, old token cannot revive.
    evidence.sessionEpoch[0] = 99;
    evidence.observationId = 0;
    SetTracking(109, 0, motion, evidence, { }, { }, nullptr, 0);
    assert(host.count == 5 && hmd.last_hmd_authority.evidence.present == 1);
    evidence.sessionEpoch[0] = 0;
    evidence.observationId = 9;
    SetTracking(110, 0, motion, evidence, { }, { }, nullptr, 0);
    assert(host.count == 6 && hmd.last_hmd_authority.evidence.present == 0);
    // Authority absent retains the same provider behavior and exactly one submit.
    SetTracking(111, 0, motion, { }, { }, { }, nullptr, 0);
    assert(host.count == 7 && hmd.last_hmd_authority.evidence.present == 0);
    check_output(hmd, motion);
    hmd.object_id = vr::k_unTrackedDeviceIndexInvalid;
    SetTracking(112, 0, motion, evidence, { }, { }, nullptr, 0);
    assert(host.count == 7);
    hmd.object_id = 42;
    // Cross-language fixture: bytes emitted by the ACTUAL Rust FFI conversion.
    if (argc == 2) {
        std::ifstream file(argv[1], std::ios::binary);
        FfiHmdAuthorityEvidenceV1 ffi { };
        file.read(reinterpret_cast<char*>(&ffi), sizeof(ffi));
        assert(file.gcount() == sizeof(ffi));
        assert(ffi.present == 1 && ffi.observationId == 0 && ffi.sourceLocateTime == -7);
        assert(ffi.sessionEpoch[0] == 0 && ffi.sessionEpoch[15] == 15 && ffi.rawValidityBits == 36);
        uint32_t positionBits[3];
        std::memcpy(positionBits, ffi.sourcePose.position, 12);
        assert(
            positionBits[0] == 0x80000000 && positionBits[1] == 1 && positionBits[2] == 0x7f7fffff
        );
        Hmd fresh;
        g_driver_provider.hmd = &fresh;
        const auto before = host.count;
        SetTracking(105, 0, motion, ffi, { }, { }, nullptr, 0);
        assert(host.count == before + 1 && fresh.last_hmd_authority.evidence.present == 1);
        assert(std::memcmp(&ffi, &fresh.last_hmd_authority.evidence, sizeof(ffi)) == 0);
        check_output(fresh, motion);
    }
    std::puts(
        "PASS actual SetTracking -> Hmd::OnPoseUpdated -> submit_pose -> fake "
        "TrackedDevicePoseUpdated; exactly one submit/call, source immutable, provider double bits "
        "exact, generic fallback"
    );
}
