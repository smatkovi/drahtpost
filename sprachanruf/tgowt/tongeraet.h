// Was beide Tongeraete gemeinsam haben.
//
// Es gibt zwei Wege, auf denen der Ton eines Anrufs laufen kann:
//
//   PulsGeraet -- direkt an PulseAudio. Der Anruf gehoert dann uns, mit
//                 allem, was daran haengt: Lautsprecher statt Hoermuschel,
//                 keine Anrufansicht, kein Naeherungssensor.
//   SipGeraet  -- ueber die SIP-Bruecke an die Telefon-App. Der Anruf
//                 gehoert dann dem System, und all das bringt es mit.
//
// Der zweite ist der richtige; der erste bleibt als Rueckfall, falls sich
// kein Telefon an der Bruecke angemeldet hat.
#pragma once

#include <string>

#include <api/scoped_refptr.h>
#include <modules/audio_device/include/audio_device_default.h>

namespace drahtpost {

class Tongeraet
    : public webrtc::webrtc_impl::AudioDeviceModuleDefault<webrtc::AudioDeviceModule> {
public:
    virtual std::string letzterFehler() const = 0;
};

} // namespace drahtpost
