// Wo das Bild der Gegenstelle liegt, bis die Oberflaeche es abholt.
//
// Der Tonprozess und die Oberflaeche sind zwei Programme -- eines in C++
// mit WebRTC, das andere Qt 4.7 auf QML. Bilder ueber einen Socket zu
// schicken hiesse, 115 kB je Bild durch den Kern zu tragen und auf beiden
// Seiten zu kopieren. Ein gemeinsames Stueck Speicher kostet das nicht.
//
// Es ist bewusst der einfachste Aufbau, der richtig sein kann: zwei
// Puffer im Wechsel und ein Zaehler, der sagt, welcher fertig ist. Der
// Schreiber fuellt immer den, der gerade nicht gelesen wird; der Leser
// schaut auf den Zaehler und liest den anderen. Ohne Sperre -- eine
// Sperre zwischen zwei Prozessen, von denen einer abstuerzen kann, waere
// schlimmer als ein Bild, das einmal zerreisst.

#pragma once

#include <atomic>
#include <cstdint>
#include <cstring>
#include <memory>
#include <string>

#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#include <api/video/video_frame.h>
#include <api/video/video_sink_interface.h>
#include <libyuv/convert_argb.h>

namespace drahtpost {

/// Groesse der Ablage: zwei Bilder bis 640x480 in RGB565.
///
/// RGB565, weil der Bildschirm des N950 das ist (/dev/fb0) und Qt es
/// ohne Umrechnung zeichnet. Die Umrechnung von I420 macht libyuv mit
/// NEON -- billiger hier als in der Oberflaeche, die dafuer keine
/// Vektoreinheit anspricht.
constexpr int kMaxBreite = 640;
constexpr int kMaxHoehe = 480;

struct Bildkopf {
    std::atomic<uint32_t> fertig;  // welcher Puffer gilt: 0 oder 1
    std::atomic<uint32_t> folge;   // steigt mit jedem Bild
    uint32_t breite;
    uint32_t hoehe;
};

constexpr size_t kPufferBytes = (size_t)kMaxBreite * kMaxHoehe * 2;
constexpr size_t kAblageBytes = sizeof(Bildkopf) + 2 * kPufferBytes;

class Bildablage {
public:
    /// Der Name im gemeinsamen Speicher. Die Oberflaeche oeffnet ihn
    /// unter demselben Namen.
    static const char *name() { return "/drahtpost-bild"; }

    bool oeffnen(const char *wie = nullptr) {
        _name = wie ? wie : name();
        int fd = shm_open(_name.c_str(), O_CREAT | O_RDWR, 0600);
        if (fd < 0) {
            return false;
        }
        if (ftruncate(fd, (off_t)kAblageBytes) != 0) {
            close(fd);
            return false;
        }
        void *p = mmap(nullptr, kAblageBytes, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        close(fd);
        if (p == MAP_FAILED) {
            return false;
        }
        _speicher = (uint8_t *)p;
        auto *kopf = (Bildkopf *)_speicher;
        kopf->fertig.store(0);
        kopf->folge.store(0);
        kopf->breite = 0;
        kopf->hoehe = 0;
        return true;
    }

    ~Bildablage() {
        if (_speicher) {
            munmap(_speicher, kAblageBytes);
        }
        shm_unlink(_name.c_str());
    }

    /// Ein Bild ablegen. Laeuft auf dem Faden, der es dekodiert hat.
    void legen(const webrtc::VideoFrame &rahmen) {
        if (!_speicher) {
            return;
        }
        const int b = rahmen.width(), h = rahmen.height();
        if (b <= 0 || h <= 0 || b > kMaxBreite || h > kMaxHoehe) {
            return;
        }
        auto *kopf = (Bildkopf *)_speicher;
        // In den Puffer schreiben, der gerade NICHT gilt.
        const uint32_t schreibe = 1 - kopf->fertig.load(std::memory_order_relaxed);
        uint8_t *ziel = _speicher + sizeof(Bildkopf) + schreibe * kPufferBytes;

        auto puffer = rahmen.video_frame_buffer()->ToI420();
        if (!puffer) {
            return;
        }
        if (libyuv::I420ToRGB565(puffer->DataY(), puffer->StrideY(),
                                 puffer->DataU(), puffer->StrideU(),
                                 puffer->DataV(), puffer->StrideV(),
                                 ziel, b * 2, b, h) != 0) {
            return;
        }
        kopf->breite = (uint32_t)b;
        kopf->hoehe = (uint32_t)h;
        // Erst die Masse, dann der Zeiger darauf: wer die Folge sieht,
        // sieht auch das fertige Bild. Release sorgt genau dafuer.
        kopf->fertig.store(schreibe, std::memory_order_release);
        kopf->folge.fetch_add(1, std::memory_order_release);
    }

    /// Die Senke, die tgcalls fuettert.
    std::shared_ptr<rtc::VideoSinkInterface<webrtc::VideoFrame>> senke() {
        if (!_senke) {
            _senke = std::make_shared<Senke>(this);
        }
        return _senke;
    }

private:
    class Senke : public rtc::VideoSinkInterface<webrtc::VideoFrame> {
    public:
        explicit Senke(Bildablage *ablage) : _ablage(ablage) {}
        void OnFrame(const webrtc::VideoFrame &rahmen) override {
            _ablage->legen(rahmen);
        }

    private:
        Bildablage *_ablage;
    };

    std::string _name;
    uint8_t *_speicher = nullptr;
    std::shared_ptr<Senke> _senke;
};

} // namespace drahtpost
