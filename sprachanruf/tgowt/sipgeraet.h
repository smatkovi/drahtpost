// Das Tongeraet, das den Ton an die Telefon-App uebergibt.
//
// Es spricht nicht mit PulseAudio, sondern mit der SIP-Bruecke, und die
// mit telepathy-sofiasip. Damit fuehrt die Telefon-App des N9/N950 das
// Gespraech: Hoermuschel statt Lautsprecher, Anrufansicht samt
// Sperrbildschirm, Naeherungssensor, Lautstaerketasten. Nichts davon
// muessen wir bauen -- nur den Ton an der richtigen Stelle abliefern.
//
// Der Takt kommt vom Telefon und von nichts anderem. Die Bruecke schreibt
// zwei Rahmen in den Socket, sobald ein RTP-Paket des Telefons ankommt --
// alle 20 ms, weil dessen Kodierer an der Tonhardware haengt. Wir lesen
// blockierend und haengen damit an derselben Uhr. Kein usleep, keine
// eigene Zeitschleife: genau daran ist die SIP-Bruecke von WhatsApp
// einmal gescheitert, und der Fehler sah von aussen aus wie ein
// Netzproblem.
//
// Ein Faden, nicht zwei. Auf jeden gelesenen Rahmen folgt genau ein
// geschriebener, im selben Durchgang: aufnehmen, abspielen, abliefern.
// Damit braucht die Gegenrichtung keinen eigenen Takt -- es geht so viel
// hinaus, wie hereinkam -- und zwei Faeden koennen nicht auseinander
// laufen.

#pragma once

#include <atomic>
#include <chrono>
#include <cstdio>
#include <cerrno>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

#include <api/make_ref_counted.h>

#include "tongeraet.h"

namespace drahtpost {

// 16 kHz: das ist, was die Bruecke liefert. G.711 auf dem Draht ist 8 kHz,
// die Bruecke rechnet hoch -- mehr als 8 kHz Bandbreite ist auf diesem Weg
// ohnehin nicht zu haben, aber 16 kHz ist die Rate, in der WebRTC und Opus
// bequem rechnen, und Umrechnen an dieser Stelle waere verschenkte
// Rechenzeit.
constexpr int kSipRate = 16000;
constexpr int kSipKanaele = 1;
constexpr int kSipRahmenMs = 10;
constexpr int kSipSamplesJeRahmen = kSipRate / 1000 * kSipRahmenMs; // 160

// Was wir WebRTC als Verzoegerung melden.
//
// Messen koennen wir sie nicht: der groesste Teil sitzt in der
// Tonhardware des Telefons, hinter sofiasip, und von dort kommt keine
// Zahl zurueck. Geschaetzt wird sie aus dem, was bekannt ist -- 20 ms
// Paketierung in jeder Richtung, dazu die Puffer der Telefon-App --, und
// eine begruendete Schaetzung ist hier besser als eine Null: an dieser
// Zahl rechnet der Jitterpuffer der Gegenstelle.
constexpr uint32_t kSipVerzugMs = 60;

class SipGeraet : public Tongeraet {
public:
    explicit SipGeraet(std::string pfad) : pfad_(std::move(pfad)) {}

    ~SipGeraet() override {
        StopRecording();
        StopPlayout();
    }

    int32_t ActiveAudioLayer(AudioLayer *ebene) const override {
        *ebene = AudioDeviceModule::kDummyAudio;
        return 0;
    }

    int32_t RegisterAudioCallback(webrtc::AudioTransport *rueckruf) override {
        weiter_.store(rueckruf);
        return 0;
    }

    int32_t Init() override { return 0; }
    bool Initialized() const override { return true; }
    int32_t Terminate() override {
        StopRecording();
        StopPlayout();
        return 0;
    }

    int32_t InitPlayout() override { return 0; }
    bool PlayoutIsInitialized() const override { return true; }
    int32_t InitRecording() override { return 0; }
    bool RecordingIsInitialized() const override { return true; }

    // Aufnahme und Wiedergabe sind hier derselbe Faden: die eine Richtung
    // taktet die andere. Gestartet wird er, sobald eine der beiden Seiten
    // ihn will, beendet, sobald keine ihn mehr will.
    int32_t StartPlayout() override {
        spielt_ = true;
        return starten();
    }
    int32_t StopPlayout() override {
        spielt_ = false;
        return beenden();
    }
    bool Playing() const override { return spielt_; }

    int32_t StartRecording() override {
        nimmt_auf_ = true;
        return starten();
    }
    int32_t StopRecording() override {
        nimmt_auf_ = false;
        return beenden();
    }
    bool Recording() const override { return nimmt_auf_; }

    int32_t SetMicrophoneMute(bool stumm) override {
        stumm_ = stumm;
        return 0;
    }
    int32_t MicrophoneMute(bool *stumm) const override {
        *stumm = stumm_;
        return 0;
    }

    std::string letzterFehler() const override { return fehler_; }

    // verbinden versucht den Socket -- getrennt vom Start, damit sich vor
    // dem Anruf pruefen laesst, ob die Bruecke ueberhaupt da ist. Ist sie
    // es nicht, faellt der Anrufer auf PulseAudio zurueck, statt ein
    // stummes Gespraech zu fuehren.
    static bool erreichbar(const std::string &pfad) {
        int s = verbinden(pfad);
        if (s < 0) {
            return false;
        }
        close(s);
        return true;
    }

private:
    static int verbinden(const std::string &pfad) {
        int s = socket(AF_UNIX, SOCK_STREAM, 0);
        if (s < 0) {
            return -1;
        }
        sockaddr_un adr{};
        adr.sun_family = AF_UNIX;
        std::strncpy(adr.sun_path, pfad.c_str(), sizeof(adr.sun_path) - 1);
        if (connect(s, (sockaddr *)&adr, sizeof(adr)) != 0) {
            close(s);
            return -1;
        }
        return s;
    }

    int32_t starten() {
        if (faden_.joinable()) {
            return 0;
        }
        laeuft_ = true;
        faden_ = std::thread([this] { schleife(); });
        return 0;
    }

    int32_t beenden() {
        if (spielt_ || nimmt_auf_) {
            return 0;
        }
        laeuft_ = false;
        // Der Faden haengt im read. Den Socket zuzumachen ist das, was ihn
        // loest -- ein Merker allein wuerde erst beim naechsten Rahmen
        // gelesen, und wenn keiner mehr kommt, nie.
        int s = draht_.exchange(-1);
        if (s >= 0) {
            shutdown(s, SHUT_RDWR);
            close(s);
        }
        if (faden_.joinable()) {
            faden_.join();
        }
        return 0;
    }

    // Lesen, aufnehmen, abspielen, schreiben -- in dieser Reihenfolge, je
    // Durchgang genau einmal.
    void schleife() {
        int s = verbinden(pfad_);
        if (s < 0) {
            fehler_ = std::string("Bruecke: ") + std::strerror(errno);
            return;
        }
        draht_.store(s);
        std::vector<int16_t> herein(kSipSamplesJeRahmen);
        std::vector<int16_t> hinaus(kSipSamplesJeRahmen * 2);
        uint64_t rahmen = 0;
        while (laeuft_) {
            if (!ganzLesen(s, herein.data(), herein.size() * 2)) {
                if (laeuft_) {
                    fehler_ = "Bruecke hat den Draht geschlossen";
                }
                break;
            }
            if (stumm_) {
                std::memset(herein.data(), 0, herein.size() * 2);
            }
            auto *w = weiter_.load();
            if (w) {
                uint32_t neu = 0;
                w->RecordedDataIsAvailable(herein.data(), kSipSamplesJeRahmen, 2,
                                           kSipKanaele, kSipRate, kSipVerzugMs, 0, 0,
                                           false, neu);
                size_t heraus = 0;
                int64_t vergangen = 0, ntp = 0;
                w->NeedMorePlayData(kSipSamplesJeRahmen, 2, kSipKanaele, kSipRate,
                                    hinaus.data(), heraus, &vergangen, &ntp);
                // Gibt WebRTC nichts her, geht Stille hinaus -- nicht
                // nichts. Sonst fiele der Takt des Telefons aus, und
                // dessen Ausgabe liefe leer.
                if (heraus != (size_t)kSipSamplesJeRahmen) {
                    std::memset(hinaus.data(), 0, kSipSamplesJeRahmen * 2);
                }
            } else {
                std::memset(hinaus.data(), 0, kSipSamplesJeRahmen * 2);
            }
            if (!ganzSchreiben(s, hinaus.data(), kSipSamplesJeRahmen * 2)) {
                if (laeuft_) {
                    fehler_ = "Bruecke nimmt nichts mehr an";
                }
                break;
            }
            takt_melden(++rahmen);
        }
        int alt = draht_.exchange(-1);
        if (alt >= 0) {
            close(alt);
        }
    }

    static bool ganzLesen(int s, void *ziel, size_t n) {
        auto *p = (uint8_t *)ziel;
        while (n > 0) {
            ssize_t k = read(s, p, n);
            if (k == 0) {
                return false;
            }
            if (k < 0) {
                if (errno == EINTR) {
                    continue;
                }
                return false;
            }
            p += k;
            n -= (size_t)k;
        }
        return true;
    }

    static bool ganzSchreiben(int s, const void *quelle, size_t n) {
        auto *p = (const uint8_t *)quelle;
        while (n > 0) {
            ssize_t k = write(s, p, n);
            if (k <= 0) {
                if (k < 0 && errno == EINTR) {
                    continue;
                }
                return false;
            }
            p += k;
            n -= (size_t)k;
        }
        return true;
    }

    // Alle fuenf Sekunden eine Zeile: laeuft der Takt, wie er soll?
    //
    // 100 Rahmen je Sekunde sind das Soll. Weicht es ab, laeuft der Ton
    // aus dem Tritt, und das ist von aussen nicht von einem Netzproblem
    // zu unterscheiden -- ohne diese Zahl haette man beim ersten Anruf
    // wieder geraten.
    void takt_melden(uint64_t rahmen) {
        if (rahmen % 500 != 0) {
            return;
        }
        auto jetzt = std::chrono::steady_clock::now();
        if (begonnen_ == std::chrono::steady_clock::time_point{}) {
            begonnen_ = jetzt;
            return;
        }
        double s = std::chrono::duration<double>(jetzt - begonnen_).count();
        begonnen_ = jetzt;
        std::fprintf(stderr, "== SIP-Ton: 500 Rahmen in %.2f s (%.1f/s, Soll 100)\n",
                     s, 500.0 / s);
        std::fflush(stderr);
    }

    std::string pfad_;
    std::atomic<webrtc::AudioTransport *> weiter_{nullptr};
    std::atomic<int> draht_{-1};
    std::atomic<bool> laeuft_{false};
    std::atomic<bool> spielt_{false};
    std::atomic<bool> nimmt_auf_{false};
    std::atomic<bool> stumm_{false};
    std::string fehler_;
    std::thread faden_;
    std::chrono::steady_clock::time_point begonnen_{};
};

inline rtc::scoped_refptr<SipGeraet> neuesSipGeraet(const std::string &pfad) {
    return rtc::make_ref_counted<SipGeraet>(pfad);
}

} // namespace drahtpost
