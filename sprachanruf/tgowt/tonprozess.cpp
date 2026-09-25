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

#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#include <third-party/json11.hpp>
#include <tgcalls/Instance.h>
#include <tgcalls/InstanceImpl.h>
#include <tgcalls/StaticThreads.h>
#include <tgcalls/v2/InstanceV2Impl.h>

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

struct Gespraech {
    std::unique_ptr<tgcalls::Instance> instanz;
    rtc::scoped_refptr<drahtpost::PulsGeraet> geraet;
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
    auto g = std::make_unique<Gespraech>();
    g->id = id;
    g->geraet = geraet;
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

int main(int argc, char **argv) {
    if (argc > 1 && std::string(argv[1]) == "--tonprobe") {
        return tonprobe(argc > 2 ? std::atoi(argv[2]) : 3);
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
