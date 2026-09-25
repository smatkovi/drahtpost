// Der Tonprozess: tgcalls, und sonst nichts.
//
// Die Drahtpost handelt den Anruf aus und reicht das Ergebnis hierher --
// Schluessel, Wege, ausgehandelte Fassung. Ab da redet dieser Prozess
// direkt mit der Gegenstelle, und die Drahtpost ist nur noch der
// Briefkasten fuer die Signalisierung des neuen Protokolls.
//
// Warum ein eigener Prozess und nicht eine Bibliothek in der Drahtpost:
// hier haengen 30 MB WebRTC und ein Dutzend Threads dran. Stuerzt der Ton
// ab, laeuft Telegram weiter -- Nachrichten kommen an, auch wenn das
// Gespraech gerade zusammengebrochen ist. Und die Drahtpost bleibt Rust.
//
//   ~/.pytelegram/ton.sock   herein: {"befehl": …}
//   ~/.pytelegram/daemon.sock hinaus: {"cmd": "call_signal", …}

#include <atomic>
#include <cstdio>
#include <cstring>
#include <chrono>
#include <cmath>
#include <condition_variable>
#include <deque>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include <signal.h>
#include <ucontext.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#include <third-party/json11.hpp>
#include <tgcalls/Instance.h>
#include <tgcalls/InstanceImpl.h>
#include <tgcalls/StaticThreads.h>
#include <rtc_base/thread.h>
#include <tgcalls/VideoCaptureInterface.h>
#include <tgcalls/platform/tdesktop/VideoCameraCapturer.h>
#include <api/video/video_frame.h>
#include <api/video/video_sink_interface.h>
#include <tgcalls/v2/InstanceV2Impl.h>

#include "bildablage.h"
#include "pulsgeraet.h"

namespace {

std::string heim() {
    const char *h = getenv("HOME");
    return h ? h : "/home/user";
}

void sagen(const std::string &was) {
    std::fprintf(stderr, "%s\n", was.c_str());
    std::fflush(stderr);
}

std::vector<uint8_t> ausHex(const std::string &s) {
    std::vector<uint8_t> aus;
    aus.reserve(s.size() / 2);
    for (size_t i = 0; i + 1 < s.size(); i += 2) {
        aus.push_back((uint8_t)std::stoi(s.substr(i, 2), nullptr, 16));
    }
    return aus;
}

std::string alsHex(const std::vector<uint8_t> &b) {
    static const char *z = "0123456789abcdef";
    std::string s;
    s.reserve(b.size() * 2);
    for (uint8_t x : b) {
        s.push_back(z[x >> 4]);
        s.push_back(z[x & 15]);
    }
    return s;
}

/// Eine Zeile an die Drahtpost -- kurz auf, kurz zu.
///
/// Eine dauerhafte Verbindung waere sparsamer, aber sie muesste den
/// Neustart der Drahtpost ueberstehen, und das ist mehr Zustand, als
/// eine Handvoll Zeilen je Gespraech wert ist.
void anDieDrahtpost(const json11::Json &was) {
    int s = socket(AF_UNIX, SOCK_STREAM, 0);
    if (s < 0) {
        return;
    }
    sockaddr_un a{};
    a.sun_family = AF_UNIX;
    std::string pfad = heim() + "/.pytelegram/daemon.sock";
    std::strncpy(a.sun_path, pfad.c_str(), sizeof(a.sun_path) - 1);
    if (connect(s, (sockaddr *)&a, sizeof(a)) == 0) {
        std::string zeile = was.dump() + "\n";
        ssize_t n = write(s, zeile.data(), zeile.size());
        (void)n;
    } else {
        sagen("⚠ Drahtpost nicht erreichbar");
    }
    close(s);
}

// --- Das laufende Gespraech -------------------------------------------

/// Die Kamera anlegen, und zwar auf dem Medienfaden von tgcalls.
///
/// Das ist keine Vorsichtsmassnahme, sondern Pflicht. Die Bildquelle
/// bekommt einen Stellvertreter (VideoTrackSourceProxy), der jeden
/// Aufruf an den Faden weiterreicht, auf dem sie angelegt wurde. Legt
/// man sie auf einem gewoehnlichen Faden an, ist dieser Faden fuer
/// WebRTC nicht vorhanden -- rtc::Thread::Current() ist dort null --,
/// und der erste Aufruf springt ins Leere:
///
///     *** Signal 11 bei pc=00000028
///     webrtc::MethodCall<VideoTrackSourceInterface, …>::Marshal(rtc::Thread*)
///
/// Der Absturz kam erst, als eine Senke dazukam; bis dahin wurde nichts
/// weitergereicht, und alles sah gut aus.
std::shared_ptr<tgcalls::VideoCaptureInterface> kameraAnlegen() {
    auto faeden = tgcalls::StaticThreads::getThreads();
    std::shared_ptr<tgcalls::VideoCaptureInterface> kamera;
    faeden->getMediaThread()->BlockingCall([&] {
        kamera = tgcalls::VideoCaptureInterface::Create(faeden, "harmattan", false, nullptr);
    });
    return kamera;
}

/// Die eigene Vorschau: dasselbe Bild, das hinausgeht, in einer zweiten
/// Ablage. Es kommt aus dem Kameramodul, nicht aus tgcalls -- dessen
/// setOutput stuerzt hier ab (siehe flicken/harmattan-kamera.patch).
std::shared_ptr<drahtpost::Bildablage> vorschauAnlegen() {
    auto ablage = std::make_shared<drahtpost::Bildablage>();
    if (!ablage->oeffnen("/drahtpost-eigenbild")) {
        return nullptr;
    }
    std::weak_ptr<drahtpost::Bildablage> schwach = ablage;
    tgcalls::kameraVorschau([schwach](const webrtc::VideoFrame &r) {
        if (auto a = schwach.lock()) {
            a->legen(r);
        }
    });
    return ablage;
}

struct Gespraech {
    std::unique_ptr<tgcalls::Instance> instanz;
    rtc::scoped_refptr<drahtpost::PulsGeraet> geraet;
    std::shared_ptr<tgcalls::VideoCaptureInterface> kamera;
    std::shared_ptr<drahtpost::Bildablage> bild;
    std::shared_ptr<drahtpost::Bildablage> eigenbild;
    int64_t id = 0;
};

std::mutex sperre;
std::unique_ptr<Gespraech> laeuft;

const char *zustandsname(tgcalls::State z) {
    switch (z) {
    case tgcalls::State::WaitInit: return "wait_init";
    case tgcalls::State::WaitInitAck: return "wait_init_ack";
    case tgcalls::State::Established: return "established";
    case tgcalls::State::Failed: return "failed";
    case tgcalls::State::Reconnecting: return "reconnecting";
    }
    return "?";
}

void auflegen() {
    std::unique_ptr<Gespraech> g;
    {
        std::lock_guard<std::mutex> l(sperre);
        g = std::move(laeuft);
    }
    if (!g) {
        return;
    }
    sagen("== Gespraech wird beendet");
    // stop() vor dem Zerstoeren, nicht danach: die Threads von WebRTC
    // laufen sonst noch, waehrend ihnen der Boden weggezogen wird.
    // Dieselbe Falle stand schon bei libtgvoip.
    if (g->instanz) {
        // Der Rueckruf von stop() kommt auf einem Faden von tgcalls und
        // kann noch eintreffen, wenn wir hier laengst weiter sind.
        // Haengt er an Stapelspeicher, schreibt er in einen Rahmen, den
        // es nicht mehr gibt -- genau das hat beim ersten Versuch den
        // Haufen zerlegt ("free(): invalid next size"). Also gehoert der
        // Zustand auf den Haufen und wird geteilt.
        struct Warten {
            std::mutex sperre;
            std::condition_variable wecker;
            bool durch = false;
        };
        auto w = std::make_shared<Warten>();
        g->instanz->stop([w](tgcalls::FinalState) {
            std::lock_guard<std::mutex> l(w->sperre);
            w->durch = true;
            w->wecker.notify_all();
        });
        std::unique_lock<std::mutex> l(w->sperre);
        if (!w->wecker.wait_for(l, std::chrono::seconds(5), [&] { return w->durch; })) {
            sagen("⚠ tgcalls hat nicht fertiggemeldet");
        }
    }
    g->instanz.reset();
    if (g->geraet) {
        g->geraet->Terminate();
    }
    tgcalls::kameraVorschau(nullptr);
    sagen("== beendet");
}

void anrufen(const json11::Json &b) {
    auflegen();

    const std::string fassung = b["fassung"].string_value();
    const auto schluesselbytes = ausHex(b["schluessel"].string_value());
    if (schluesselbytes.size() != tgcalls::EncryptionKey::kSize) {
        sagen("✗ Schluessel hat die falsche Laenge");
        return;
    }
    auto schluessel = std::make_shared<std::array<uint8_t, tgcalls::EncryptionKey::kSize>>();
    std::memcpy(schluessel->data(), schluesselbytes.data(), schluesselbytes.size());

    // Descriptor hat keinen leeren Grundzustand: der Schluessel gehoert
    // dazu, seit es ihn gibt. Das ist Absicht -- ein Gespraech ohne
    // Schluessel soll sich gar nicht erst beschreiben lassen.
    tgcalls::Descriptor d{
        .encryptionKey = tgcalls::EncryptionKey(schluessel, b["ausgehend"].bool_value()),
    };
    d.version = fassung;
    d.config.initializationTimeout = 30.;
    d.config.receiveTimeout = 20.;
    d.config.enableP2P = b["p2p_erlaubt"].bool_value();
    d.config.maxApiLayer = 92;
    // Die Aufbereitung von WebRTC bleibt aus: sie kostet auf diesem
    // Geraet zwei Drittel eines Kerns, und der Ton geht ohnehin durch
    // Nokias eigene. Gemessen steht es in sprachanruf/README.md.
    d.config.enableAEC = false;
    d.config.enableNS = false;
    d.config.enableAGC = false;
    d.initialNetworkType = tgcalls::NetworkType::WiFi;

    for (const auto &w : b["wege"].array_items()) {
        const std::string art = w["art"].string_value();
        if (art == "webrtc") {
            tgcalls::RtcServer s;
            s.id = (uint8_t)w["id"].int_value();
            s.host = w["ip"].string_value();
            s.port = (uint16_t)w["port"].int_value();
            s.login = w["benutzer"].string_value();
            s.password = w["passwort"].string_value();
            s.isTurn = w["turn"].bool_value();
            d.rtcServers.push_back(s);
        } else {
            tgcalls::Endpoint e;
            e.endpointId = (int64_t)w["id"].number_value();
            e.host.ipv4 = w["ip"].string_value();
            e.host.ipv6 = w["ipv6"].string_value();
            e.port = (uint16_t)w["port"].int_value();
            e.type = w["tcp"].bool_value() ? tgcalls::EndpointType::TcpRelay
                                           : tgcalls::EndpointType::UdpRelay;
            const auto marke = ausHex(w["peer_tag"].string_value());
            std::memcpy(e.peerTag, marke.data(), std::min<size_t>(marke.size(), 16));
            d.endpoints.push_back(e);
        }
    }

    auto geraet = drahtpost::neuesGeraet("", "");
    d.createAudioDeviceModule =
        [geraet](webrtc::TaskQueueFactory *) { return geraet; };

    // Das Bild der Gegenstelle landet in einer Ablage, aus der die
    // Oberflaeche es sich holt. Auch bei einem Sprachanruf angelegt:
    // Telegram erlaubt, die Kamera mitten im Gespraech einzuschalten,
    // und dann ist es zu spaet, den Weg erst zu bauen.
    auto bild = std::make_shared<drahtpost::Bildablage>();
    bild->oeffnen();

    std::shared_ptr<tgcalls::VideoCaptureInterface> kamera;
    std::shared_ptr<drahtpost::Bildablage> eigenbild;
    if (b["video"].bool_value()) {
        kamera = kameraAnlegen();
        d.videoCapture = kamera;
        eigenbild = vorschauAnlegen();
    }

    const int64_t id = (int64_t)b["id"].number_value();
    d.stateUpdated = [id](tgcalls::State z) {
        sagen(std::string("== Zustand: ") + zustandsname(z));
        anDieDrahtpost(json11::Json::object{
            {"cmd", "call_state"},
            {"args", json11::Json::object{{"call_id", (double)id},
                                          {"state", zustandsname(z)}}},
        });
    };
    d.signalingDataEmitted = [](const std::vector<uint8_t> &daten) {
        // Laeuft auf dem Netzfaden von tgcalls. Der Schreibvorgang ist
        // kurz und geht an einen lokalen Socket -- aber blockieren darf
        // er dort trotzdem nicht, also in einen eigenen Faden.
        auto kopie = daten;
        std::thread([kopie] {
            anDieDrahtpost(json11::Json::object{
                {"cmd", "call_signal"},
                {"args", json11::Json::object{{"data", alsHex(kopie)}}},
            });
        }).detach();
    };

    sagen("== Anruf " + std::to_string(id) + ", Fassung " + fassung + ", " +
          std::to_string(d.rtcServers.size()) + " WebRTC-Wege, " +
          std::to_string(d.endpoints.size()) + " Reflektoren");

    auto instanz = tgcalls::Meta::Create(fassung, std::move(d));
    if (!instanz) {
        sagen("✗ tgcalls kennt die Fassung " + fassung + " nicht");
        return;
    }
    instanz->setIncomingVideoOutput(bild->senke());

    auto g = std::make_unique<Gespraech>();
    g->id = id;
    g->geraet = geraet;
    g->kamera = kamera;
    g->bild = bild;
    g->eigenbild = eigenbild;
    g->instanz = std::move(instanz);
    std::lock_guard<std::mutex> l(sperre);
    laeuft = std::move(g);
}

void signal(const json11::Json &b) {
    std::lock_guard<std::mutex> l(sperre);
    if (!laeuft || !laeuft->instanz) {
        return;
    }
    laeuft->instanz->receiveSignalingData(ausHex(b["daten"].string_value()));
}

void zeile_verarbeiten(const std::string &zeile) {
    std::string fehler;
    auto j = json11::Json::parse(zeile, fehler);
    if (!fehler.empty()) {
        sagen("⚠ unverstaendliche Zeile: " + fehler);
        return;
    }
    const std::string befehl = j["befehl"].string_value();
    if (befehl == "anrufen") {
        anrufen(j["gespraech"]);
    } else if (befehl == "signal") {
        signal(j);
    } else if (befehl == "auflegen") {
        auflegen();
    } else if (befehl == "probe") {
        // Nur ein Lebenszeichen. Wer fragt, will wissen, ob der Weg vom
        // Daemon hierher steht -- nicht, wie es dem Ton geht.
        sagen("== Probe: Tonprozess ist da");
    } else if (befehl == "kamera") {
        // Die Kamera mitten im Gespraech: Telegram nennt es Call
        // Upgrade. Die Signalisierung macht tgcalls selbst.
        std::lock_guard<std::mutex> l(sperre);
        if (!laeuft || !laeuft->instanz) {
            return;
        }
        if (j["an"].bool_value()) {
            if (!laeuft->kamera) {
                laeuft->kamera = kameraAnlegen();
                laeuft->eigenbild = vorschauAnlegen();
            }
            laeuft->kamera->setState(tgcalls::VideoState::Active);
            laeuft->instanz->setVideoCapture(laeuft->kamera);
            sagen("== Kamera an");
        } else if (laeuft->kamera) {
            laeuft->kamera->setState(tgcalls::VideoState::Inactive);
            sagen("== Kamera aus");
        }
    } else if (befehl == "stumm") {
        std::lock_guard<std::mutex> l(sperre);
        if (laeuft && laeuft->instanz) {
            laeuft->instanz->setMuteMicrophone(j["an"].bool_value());
        }
    } else {
        sagen("⚠ unbekannter Befehl: " + befehl);
    }
}

/// Der Tonweg allein, ohne Anruf.
///
/// Was hier gemessen wird, ist das, woran die SIP-Bruecke gescheitert
/// ist: ob die Rahmen gleichmaessig kommen. Ein Mittelwert von 10 ms bei
/// grosser Streuung waere kein gutes Zeichen -- dann holpert der Ton,
/// auch wenn die Summe stimmt.
class Zaehltransport : public webrtc::AudioTransport {
public:
    int32_t RecordedDataIsAvailable(const void *, size_t n, size_t, size_t,
                                    uint32_t, uint32_t verzug, int32_t, uint32_t,
                                    bool, uint32_t &neu) override {
        neu = 0;
        auto jetzt = std::chrono::steady_clock::now();
        if (aufgenommen_ > 0) {
            double ms = std::chrono::duration<double, std::milli>(jetzt - zuletzt_).count();
            summe_ += ms;
            if (ms > groesster_) {
                groesster_ = ms;
            }
            if (ms < kleinster_) {
                kleinster_ = ms;
            }
        }
        zuletzt_ = jetzt;
        ++aufgenommen_;
        samples_ += n;
        verzug_ = verzug;
        return 0;
    }

    int32_t NeedMorePlayData(size_t n, size_t, size_t, uint32_t, void *ton,
                             size_t &heraus, int64_t *vergangen,
                             int64_t *ntp) override {
        // Ein leiser Ton, damit man beim Zuhoeren merkt, ob der Weg
        // wirklich bis zum Lautsprecher geht.
        auto *p = (int16_t *)ton;
        for (size_t i = 0; i < n; ++i) {
            double t = (phase_ + i) / 48000.0;
            p[i] = (int16_t)(2000 * std::sin(2 * M_PI * 440 * t));
        }
        phase_ += n;
        heraus = n;
        *vergangen = 0;
        *ntp = 0;
        ++gespielt_;
        return 0;
    }

    void PullRenderData(int, int, size_t, size_t, void *, int64_t *, int64_t *) override {}

    void bericht() const {
        std::printf("Aufnahme:   %d Rahmen, %zu Samples, Verzug %u ms\n",
                    aufgenommen_, samples_, verzug_);
        if (aufgenommen_ > 1) {
            std::printf("Abstand:    %.2f ms im Mittel, %.2f bis %.2f\n",
                        summe_ / (aufgenommen_ - 1), kleinster_, groesster_);
        }
        std::printf("Wiedergabe: %d Rahmen\n", gespielt_);
    }

private:
    int aufgenommen_ = 0;
    int gespielt_ = 0;
    size_t samples_ = 0;
    size_t phase_ = 0;
    uint32_t verzug_ = 0;
    double summe_ = 0, groesster_ = 0, kleinster_ = 1e9;
    std::chrono::steady_clock::time_point zuletzt_;
};

/// Die Kamera allein, ohne Anruf.
///
/// Zaehlt, was aus der GStreamer-Kette wirklich herauskommt, und was das
/// Umrechnen nach I420 kostet. Ohne diesen Weg zeigte sich ein Fehler in
/// der Kette erst im Gespraech -- als schwarzes Bild beim Gegenueber.
class Bildzaehler : public rtc::VideoSinkInterface<webrtc::VideoFrame> {
public:
    void OnFrame(const webrtc::VideoFrame &rahmen) override {
        auto jetzt = std::chrono::steady_clock::now();
        if (bilder > 0) {
            summe_ += std::chrono::duration<double, std::milli>(jetzt - zuletzt_).count();
        }
        zuletzt_ = jetzt;
        ++bilder;
        breite = rahmen.width();
        hoehe = rahmen.height();
    }
    double abstand() const { return bilder > 1 ? summe_ / (bilder - 1) : 0; }
    int bilder = 0, breite = 0, hoehe = 0;

private:
    double summe_ = 0;
    std::chrono::steady_clock::time_point zuletzt_;
};

/// Dieselbe Kamera, aber ohne tgcalls dazwischen.
///
/// Wenn die Probe hier laeuft und mit tgcalls nicht, liegt es nicht an
/// der Kamera. Das zu trennen ist die halbe Fehlersuche.
int rohkameraprobe(int sekunden) {
    auto modul = tgcalls::harmattanKameraOeffnen();
    if (!modul) {
        sagen("✗ kein Modul");
        return 1;
    }
    auto zaehler = std::make_shared<Bildzaehler>();
    modul->RegisterCaptureDataCallback(zaehler.get());
    webrtc::VideoCaptureCapability wunsch;
    wunsch.width = 320;
    wunsch.height = 240;
    wunsch.maxFPS = 15;
    wunsch.videoType = webrtc::VideoType::kI420;
    if (modul->StartCapture(wunsch) != 0) {
        sagen("✗ StartCapture");
        return 1;
    }
    std::this_thread::sleep_for(std::chrono::seconds(sekunden));
    modul->StopCapture();
    modul->DeRegisterCaptureDataCallback();
    std::printf("Rohkamera: %d Bilder, %dx%d, %.1f ms Abstand (= %.1f B/s)\n",
                zaehler->bilder, zaehler->breite, zaehler->hoehe,
                zaehler->abstand(), zaehler->abstand() > 0 ? 1000.0 / zaehler->abstand() : 0);
    return 0;
}

int kameraprobe(int sekunden, bool ohneSenke = false) {
    auto kamera = kameraAnlegen();
    if (!kamera) {
        sagen("✗ keine Kamera");
        return 1;
    }
    auto zaehler = std::make_shared<Bildzaehler>();
    // Mit "--kameraprobe 5 ohne" laeuft die Kette ohne eigene Senke --
    // damit laesst sich trennen, ob das Zeigen oder das Aufnehmen
    // schiefgeht.
    // Gezaehlt wird am eigenen Haken, nicht ueber setOutput: dessen
    // Stellvertreter stuerzt hier ab (siehe flicken/harmattan-kamera).
    if (!ohneSenke) {
        tgcalls::kameraVorschau([zaehler](const webrtc::VideoFrame &r) {
            zaehler->OnFrame(r);
        });
    }
    kamera->setState(tgcalls::VideoState::Active);
    std::this_thread::sleep_for(std::chrono::seconds(sekunden));
    kamera->setState(tgcalls::VideoState::Inactive);
    if (ohneSenke) {
        sagen("== ohne eigene Senke durchgelaufen");
        return 0;
    }
    if (zaehler->bilder == 0) {
        sagen("✗ kein einziges Bild -- Rechte auf /dev/media0? gst-launch-0.10 da?");
        return 1;
    }
    std::printf("Kamera: %d Bilder in %d s, %dx%d, %.1f ms Abstand (= %.1f B/s)\n",
                zaehler->bilder, sekunden, zaehler->breite, zaehler->hoehe,
                zaehler->abstand(), 1000.0 / zaehler->abstand());
    return 0;
}

int tonprobe(int sekunden) {
    auto geraet = drahtpost::neuesGeraet("", "");
    Zaehltransport zaehler;
    geraet->RegisterAudioCallback(&zaehler);
    if (geraet->StartPlayout() != 0 || geraet->StartRecording() != 0) {
        sagen("✗ " + geraet->letzterFehler());
        return 1;
    }
    std::this_thread::sleep_for(std::chrono::seconds(sekunden));
    geraet->StopRecording();
    geraet->StopPlayout();
    if (!geraet->letzterFehler().empty()) {
        sagen("⚠ " + geraet->letzterFehler());
    }
    zaehler.bericht();
    // In Sekunden gerechnet: 100 Rahmen je Sekunde sind der Sollwert.
    return 0;
}

} // namespace

/// Bei einem Absturz wenigstens sagen, wo.
///
/// Auf dem N950 gibt es kein gdb, und ein "Segmentation fault" ohne
/// weitere Angabe ist nicht mehr als die Feststellung, dass etwas
/// schiefging. backtrace() aus der glibc reicht hier: es geht nur um die
/// Frage, in wessen Code es knallt.
void absturzmelder(int nr, siginfo_t *info, void *zusatz) {
    // backtrace() kommt auf ARM nicht durch den Signalrahmen -- der
    // Ruecklauf endete immer nach zwei Zeilen. Der Programmzeiger steht
    // aber im Zusatz, und mit ihm und der unabgespeckten Datei sagt
    // `addr2line` genau, wo es knallt.
    auto *u = (ucontext_t *)zusatz;
    char zeile[160];
    int n = snprintf(zeile, sizeof(zeile),
                     "\n*** Signal %d bei pc=%08lx lr=%08lx, Adresse %p\n",
                     nr, (unsigned long)u->uc_mcontext.arm_pc,
                     (unsigned long)u->uc_mcontext.arm_lr, info->si_addr);
    n += snprintf(zeile + n, sizeof(zeile) - n, "    Melder liegt bei %p\n",
                  (void *)&absturzmelder);
    ssize_t x = write(2, zeile, (size_t)n);
    (void)x;
    _exit(128 + nr);
}

int main(int argc, char **argv) {
    struct sigaction sa {};
    sa.sa_sigaction = absturzmelder;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGSEGV, &sa, nullptr);
    sigaction(SIGABRT, &sa, nullptr);
    sigaction(SIGBUS, &sa, nullptr);
    if (argc > 1 && std::string(argv[1]) == "--tonprobe") {
        return tonprobe(argc > 2 ? std::atoi(argv[2]) : 3);
    }
    if (argc > 1 && std::string(argv[1]) == "--rohkamera") {
        return rohkameraprobe(argc > 2 ? std::atoi(argv[2]) : 5);
    }
    if (argc > 1 && std::string(argv[1]) == "--kameraprobe") {
        return kameraprobe(argc > 2 ? std::atoi(argv[2]) : 5,
                           argc > 3 && std::string(argv[3]) == "ohne");
    }
    tgcalls::Register<tgcalls::InstanceImpl>();
    tgcalls::Register<tgcalls::InstanceV2Impl>();

    const std::string pfad = heim() + "/.pytelegram/ton.sock";
    unlink(pfad.c_str());
    int horcher = socket(AF_UNIX, SOCK_STREAM, 0);
    if (horcher < 0) {
        sagen("✗ Socket: kein Platz");
        return 1;
    }
    sockaddr_un a{};
    a.sun_family = AF_UNIX;
    std::strncpy(a.sun_path, pfad.c_str(), sizeof(a.sun_path) - 1);
    if (bind(horcher, (sockaddr *)&a, sizeof(a)) < 0 || listen(horcher, 4) < 0) {
        sagen("✗ Socket " + pfad + " laesst sich nicht oeffnen");
        return 1;
    }
    chmod(pfad.c_str(), 0600);
    sagen("== Tonprozess horcht auf " + pfad);

    // Ein Verbinder nach dem anderen, und je Verbindung Zeile fuer
    // Zeile. Mehr braucht es nicht: es redet genau eine Drahtpost mit
    // uns, und sie schickt eine Handvoll Zeilen je Gespraech.
    for (;;) {
        int c = accept(horcher, nullptr, nullptr);
        if (c < 0) {
            continue;
        }
        std::string rest;
        char puffer[4096];
        ssize_t n;
        while ((n = read(c, puffer, sizeof(puffer))) > 0) {
            rest.append(puffer, (size_t)n);
            size_t p;
            while ((p = rest.find('\n')) != std::string::npos) {
                std::string zeile = rest.substr(0, p);
                rest.erase(0, p + 1);
                if (!zeile.empty()) {
                    zeile_verarbeiten(zeile);
                }
            }
        }
        close(c);
    }
}
