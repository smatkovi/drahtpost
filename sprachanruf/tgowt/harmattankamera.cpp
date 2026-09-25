// Die Kamera des N950 als WebRTC-Aufnahmegeraet.
//
// WebRTCs eigener V4L2-Weg funktioniert hier nicht. Der N950 hat keine
// einfache Aufnahmekarte, sondern den OMAP3-ISP mit Media-Controller:
// /dev/media0 und ein Dutzend Subdev-Knoten, die erst zu einer Kette
// verschaltet werden muessen, bevor irgendwo Bilder herauskommen.
// video_capture_v4l2 setzt einem Geraet das Format und startet es -- das
// geht hier ins Leere.
//
// Verschalten kann das Harmattans eigenes GStreamer-Element subdevsrc
// ("MC camera source"). Wir starten also eine kurze Kette als eigenen
// Prozess und lesen die Bilder aus einer Roehre:
//
//   subdevsrc ! video/x-raw-yuv,format=UYVY,width=W,height=H ! fdsink
//
// Wichtig ist, dass W und H schon dort stehen: der ISP skaliert dann in
// Hardware. Ohne diese Angabe liefert die Kamera 1008x754, und das
// Herunterrechnen kostet Rechenzeit, die der Kodierer braucht.
//
// Warum ein eigener Prozess und nicht GStreamer im Programm: dieses
// Programm ist mit clang und WebRTCs libc++ gebaut, GStreamer 0.10 ist
// GCC 4.4 und glib. Eine Roehre kennt diesen Streit nicht.

#include <atomic>
#include <cstdio>
#include <cstdarg>
#include <functional>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

#include <signal.h>
#include <spawn.h>
#include <sys/wait.h>
#include <unistd.h>

#include <api/make_ref_counted.h>
#include <api/video/i420_buffer.h>
#include <api/video/video_frame.h>
#include <modules/video_capture/video_capture.h>
#include <rtc_base/logging.h>
#include <cstdio>
#include <cstdarg>
#include <functional>
#include <rtc_base/time_utils.h>
#include <libyuv/convert.h>

extern char **environ;

namespace tgcalls {

/// Wohin jedes aufgenommene Bild zusaetzlich geht -- die Vorschau.
///
/// tgcalls hat dafuer setOutput, und das stuerzt hier ab: der
/// Stellvertreter der Bildquelle reicht den Aufruf an einen Faden
/// weiter, der nicht da ist (MethodCall<...>::Marshal, Sprung nach
/// 0x208). Wir brauchen den Umweg aber nicht -- an dieser Stelle haben
/// wir das Bild ohnehin in der Hand, und zwar bevor tgcalls es
/// zuschneidet.
static std::function<void(const webrtc::VideoFrame &)> g_vorschau;

void kameraVorschau(std::function<void(const webrtc::VideoFrame &)> wohin) {
    g_vorschau = std::move(wohin);
}

namespace {

/// Spuren nur, wenn DRAHTPOST_KAMERASPUR gesetzt ist. Im Anruf sind sie
/// Ballast; bei der Fehlersuche waren sie das Einzige, was half.
void spur(const char *form, ...) {
    static const bool an = getenv("DRAHTPOST_KAMERASPUR") != nullptr;
    if (!an) {
        return;
    }
    va_list rest;
    va_start(rest, form);
    std::fprintf(stderr, "[kamera] ");
    std::vfprintf(stderr, form, rest);
    std::fprintf(stderr, "\n");
    std::fflush(stderr);
    va_end(rest);
}

class HarmattanKamera : public webrtc::VideoCaptureModule {
public:
    void RegisterCaptureDataCallback(
        rtc::VideoSinkInterface<webrtc::VideoFrame> *rueckruf) override {
        _senke = rueckruf;
    }
    void RegisterCaptureDataCallback(webrtc::RawVideoSinkInterface *) override {}
    void DeRegisterCaptureDataCallback() override { _senke = nullptr; }

    int32_t StartCapture(const webrtc::VideoCaptureCapability &wunsch) override {
        spur("StartCapture %dx%d", wunsch.width, wunsch.height);
        if (_laeuft) {
            return 0;
        }
        _breite = wunsch.width > 0 ? wunsch.width : 320;
        _hoehe = wunsch.height > 0 ? wunsch.height : 240;
        _bildrate = wunsch.maxFPS > 0 ? wunsch.maxFPS : 15;

        int roehre[2];
        if (pipe(roehre) != 0) {
            return -1;
        }
        // Die Kette bekommt die Roehre als Standardausgabe. fdsink mit
        // fd=1 ist einfacher als appsink und braucht keine Bibliothek.
        std::string kappen = "video/x-raw-yuv,format=(fourcc)UYVY,width=" +
                             std::to_string(_breite) + ",height=" +
                             std::to_string(_hoehe);
        const char *befehl[] = {"gst-launch-0.10", "subdevsrc",  "!",
                                kappen.c_str(),    "!",          "fdsink",
                                "fd=1",            nullptr};

        posix_spawn_file_actions_t taten;
        posix_spawn_file_actions_init(&taten);
        posix_spawn_file_actions_adddup2(&taten, roehre[1], 1);
        posix_spawn_file_actions_addclose(&taten, roehre[0]);
        posix_spawn_file_actions_addclose(&taten, roehre[1]);
        int fehler = posix_spawnp(&_kind, "gst-launch-0.10", &taten, nullptr,
                                  (char *const *)befehl, environ);
        posix_spawn_file_actions_destroy(&taten);
        close(roehre[1]);
        if (fehler != 0) {
            close(roehre[0]);
            RTC_LOG(LS_ERROR) << "gst-launch-0.10 startet nicht: " << fehler;
            return -1;
        }
        spur("gst-launch gestartet, pid %d", (int)_kind);
        _fd = roehre[0];
        _laeuft = true;
        _leser = std::thread([this] { leseschleife(); });
        return 0;
    }

    int32_t StopCapture() override {
        if (!_laeuft) {
            return 0;
        }
        _laeuft = false;
        if (_kind > 0) {
            kill(_kind, SIGTERM);
        }
        // Erst den Kindprozess beenden, dann die Roehre schliessen: der
        // Lesefaden haengt in read() und kommt nur heraus, wenn das
        // andere Ende zugeht.
        if (_leser.joinable()) {
            _leser.join();
        }
        if (_kind > 0) {
            int stand = 0;
            waitpid(_kind, &stand, 0);
            _kind = -1;
        }
        if (_fd >= 0) {
            close(_fd);
            _fd = -1;
        }
        return 0;
    }

    const char *CurrentDeviceName() const override { return "harmattan"; }
    bool CaptureStarted() override { return _laeuft; }

    int32_t CaptureSettings(webrtc::VideoCaptureCapability &was) override {
        was.width = _breite;
        was.height = _hoehe;
        was.maxFPS = _bildrate;
        was.videoType = webrtc::VideoType::kI420;
        return 0;
    }

    int32_t SetCaptureRotation(webrtc::VideoRotation drehung) override {
        _drehung = drehung;
        return 0;
    }
    bool SetApplyRotation(bool) override { return true; }
    bool GetApplyRotation() override { return true; }

private:
    void leseschleife() {
        // UYVY ist zwei Bytes je Bildpunkt. Ein Bild kommt am Stueck --
        // aber nicht in einem read: die Roehre gibt her, was da ist, und
        // ein halbes Bild waere ein zerrissenes.
        const size_t roh_groesse = (size_t)_breite * _hoehe * 2;
        std::vector<uint8_t> roh(roh_groesse);
        while (_laeuft) {
            size_t habe = 0;
            while (habe < roh_groesse && _laeuft) {
                ssize_t n = read(_fd, roh.data() + habe, roh_groesse - habe);
                if (n <= 0) {
                    _laeuft = false;
                    break;
                }
                habe += (size_t)n;
            }
            if (habe < roh_groesse) {
                break;
            }
            auto *senke = _senke.load();
            if (!senke) {
                continue;
            }
            auto bild = webrtc::I420Buffer::Create(_breite, _hoehe);
            // libyuv liegt schon in libtg_owt -- und es rechnet mit NEON.
            if (libyuv::UYVYToI420(roh.data(), _breite * 2,
                                   bild->MutableDataY(), bild->StrideY(),
                                   bild->MutableDataU(), bild->StrideU(),
                                   bild->MutableDataV(), bild->StrideV(),
                                   _breite, _hoehe) != 0) {
                continue;
            }
            if (_zaehler == 0) {
                spur("erstes Bild %dx%d", _breite, _hoehe);
            }
            ++_zaehler;
            auto rahmen = webrtc::VideoFrame::Builder()
                              .set_video_frame_buffer(bild)
                              .set_rotation(_drehung)
                              .set_timestamp_us(rtc::TimeMicros())
                              .build();
            if (g_vorschau) {
                g_vorschau(rahmen);
            }
            senke->OnFrame(rahmen);
        }
    }

    std::atomic<rtc::VideoSinkInterface<webrtc::VideoFrame> *> _senke{nullptr};
    std::atomic<bool> _laeuft{false};
    std::thread _leser;
    pid_t _kind = -1;
    int _fd = -1;
    int _breite = 320, _hoehe = 240, _bildrate = 15;
    int64_t _zaehler = 0;
    webrtc::VideoRotation _drehung = webrtc::kVideoRotation_0;
};

} // namespace

/// Der Haken, den VideoCameraCapturer aufruft, bevor er es mit WebRTCs
/// eigenem V4L2-Weg versucht.
rtc::scoped_refptr<webrtc::VideoCaptureModule> harmattanKameraOeffnen() {
    spur("Haken gerufen");
    if (access("/dev/media0", R_OK | W_OK) != 0) {
        // Ohne Rechte auf den Media-Controller waere die Kette still --
        // dann lieber gleich sagen, woran es liegt.
        RTC_LOG(LS_ERROR) << "/dev/media0 nicht lesbar -- fehlt die Gruppe video?";
        return nullptr;
    }
    return rtc::make_ref_counted<HarmattanKamera>();
}

} // namespace tgcalls
