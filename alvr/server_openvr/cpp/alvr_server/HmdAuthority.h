#pragma once
#include "bindings.h"
#include "openvr_driver_wrap.h"
#include <array>
#include <cstddef>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>

static_assert(sizeof(FfiHmdAuthorityEvidenceV1) == 144);
static_assert(alignof(FfiHmdAuthorityEvidenceV1) == 8);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, present) == 0);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, sessionEpoch) == 4);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, rawValidityBits) == 20);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, observationId) == 24);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, referenceSpaceGeneration) == 32);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, sourceLocateTime) == 40);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, sourcePose) == 48);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, reserved0) == 76);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, selectedTrackingTimestamp) == 80);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, predictionFromTimestamp) == 88);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, predictionTargetTimestamp) == 96);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, serverRecenterEpoch) == 104);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, inverseRecenteringOrigin) == 112);
static_assert(offsetof(FfiHmdAuthorityEvidenceV1, reserved1) == 140);

// Prevent multiple authority associations for a selected observation, without suppressing
// any ordinary pose submission. A repeated nearest-newer result remains generic tracking.
class HmdAuthoritySubmitGate {
    struct Entry {
        FfiHmdAuthorityEvidenceV1 evidence;
        bool retired;
    };
    std::map<std::array<uint8_t, 16>, Entry> entries;
    std::array<uint8_t, 16> active { };
    bool hasActive = false;

public:
    bool accept(const FfiHmdAuthorityEvidenceV1& e) {
        if (e.present != 1 || e.observationId > INT64_MAX || e.referenceSpaceGeneration > INT64_MAX)
            return false;
        std::array<uint8_t, 16> token;
        std::memcpy(token.data(), e.sessionEpoch, token.size());
        auto it = entries.find(token);
        if (it != entries.end()) {
            auto& old = it->second;
            if (old.retired || token != active || e.observationId < old.evidence.observationId)
                return false;
            if (e.observationId == old.evidence.observationId) {
                if (e.referenceSpaceGeneration != old.evidence.referenceSpaceGeneration
                    || e.sourceLocateTime != old.evidence.sourceLocateTime
                    || e.rawValidityBits != old.evidence.rawValidityBits
                    || std::memcmp(&e.sourcePose, &old.evidence.sourcePose, sizeof(FfiPose)) != 0)
                    old.retired = true;
                return false;
            }
            if (e.referenceSpaceGeneration < old.evidence.referenceSpaceGeneration)
                return false;
            old.evidence = e;
        } else {
            if (hasActive)
                entries.at(active).retired = true;
            active = token;
            hasActive = true;
            entries.emplace(token, Entry { e, false });
        }
        return true;
    }
};

// Exact DriverPose doubles are retained; source floats are never overwritten by output.
struct HmdAuthorityAssociationV1 {
    FfiHmdAuthorityEvidenceV1 evidence { };
    vr::DriverPose_t providerOutput { };
};

inline void TraceHmdAuthority(const HmdAuthorityAssociationV1& association) {
    static const bool enabled = [] {
#ifdef _WIN32
        char* value = nullptr;
        size_t length = 0;
        if (_dupenv_s(&value, &length, "MONAKA_ALVR_HMD_AUTHORITY_TRACE") != 0)
            return false;
        const bool result = value && std::strcmp(value, "1") == 0;
        std::free(value);
        return result;
#else
        const auto* value = std::getenv("MONAKA_ALVR_HMD_AUTHORITY_TRACE");
        return value && std::strcmp(value, "1") == 0;
#endif
    }();
    if (!enabled)
        return;
    const auto& e = association.evidence;
    const auto& p = association.providerOutput;
    uint32_t source[7];
    std::memcpy(source, e.sourcePose.position, 12);
    std::memcpy(source + 3, &e.sourcePose.orientation, 16);
    const double output[] = { p.vecPosition[0], p.vecPosition[1], p.vecPosition[2], p.qRotation.x,
                              p.qRotation.y,    p.qRotation.z,    p.qRotation.w };
    uint64_t bits[7];
    std::memcpy(bits, output, sizeof(bits));
    std::fprintf(stderr, "MONAKA_ALVR_HMD_AUTHORITY_V1 provider present=%u session=", e.present);
    for (auto byte : e.sessionEpoch)
        std::fprintf(stderr, "%02x", byte);
    std::fprintf(
        stderr,
        " obs=%llu gen=%llu locate=%lld selected=%llu predictionFrom=%llu target=%llu "
        "recenter=%llu flags=%u sourceBits=",
        static_cast<unsigned long long>(e.observationId),
        static_cast<unsigned long long>(e.referenceSpaceGeneration),
        static_cast<long long>(e.sourceLocateTime),
        static_cast<unsigned long long>(e.selectedTrackingTimestamp),
        static_cast<unsigned long long>(e.predictionFromTimestamp),
        static_cast<unsigned long long>(e.predictionTargetTimestamp),
        static_cast<unsigned long long>(e.serverRecenterEpoch),
        e.rawValidityBits
    );
    for (auto bit : source)
        std::fprintf(stderr, "%08x,", bit);
    std::fprintf(stderr, " providerBits=");
    for (auto bit : bits)
        std::fprintf(stderr, "%016llx,", static_cast<unsigned long long>(bit));
    std::fprintf(stderr, " submitAssociation=one\n");
}
