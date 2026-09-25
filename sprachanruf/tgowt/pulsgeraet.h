// Das Tongeraet fuer WebRTC auf der N950: geradewegs an PulseAudio.
//
// Das ist der Rueckfall, nicht der Regelweg -- der laeuft seit der
// SIP-Bruecke ueber die Telefon-App (sipgeraet.h). Hierher kommt man nur,
// wenn sich kein Telefon an der Bruecke angemeldet hat; dann telefoniert
// man wie zuvor, also ueber den Lautsprecher und ohne Anrufansicht.
//
// tg_owt ist mit TG_OWT_BUILD_AUDIO_BACKENDS=OFF gebaut -- absichtlich,
// die eigenen Hintergruende von WebRTC sind laut den Entwicklern selbst
// nur zum Vorfuehren gedacht. Wer sie weglaesst, muss aber selbst eines
// mitbringen, sonst bekommt tgcalls eine Attrappe: der Anruf kommt
// zustande und bleibt in beide Richtungen still. Das waere von aussen
// nicht vom urspruenglichen Fehler zu unterscheiden.
//
// Der Takt kommt vom Ton und von nichts anderem. pa_simple_read blockiert,
// bis 10 ms Mikrofon da sind; pa_simple_write blockiert, bis 10 ms Platz
// im Puffer ist. Kein usleep, keine eigene Uhr. Genau das war der Fehler
// in der SIP-Bruecke: wer in der Abholschleife schlaeft, haelt sie an.

#pragma once

#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

#include <cerrno>
#include <cstring>
#include <pthread.h>
#include <sched.h>
#include <sys/resource.h>
#include <sys/time.h>

#include <pulse/error.h>
#include <pulse/simple.h>

#include <api/make_ref_counted.h>

#include "tongeraet.h"

namespace drahtpost {

// Die Hardware der N950 laeuft auf 48 kHz (module-alsa-card-old,
// rate=48000), und WebRTC rechnet von Haus aus in 48 kHz. Also kein
// Umrechnen dazwischen -- jede Umrechnung kostet Rechenzeit und
// Genauigkeit, und beides ist hier knapp.
constexpr int kRate = 48000;
constexpr int kKanaele = 1;
constexpr int kRahmenMs = 10;
constexpr int kSamplesJeRahmen = kRate / 1000 * kRahmenMs; // 480

class PulsGeraet : public Tongeraet {
public:
    PulsGeraet(std::string quelle, std::string senke)
        : quelle_(std::move(quelle)), senke_(std::move(senke)) {}

    ~PulsGeraet() override {
        StopRecording();
        StopPlayout();
    }

    int32_t ActiveAudioLayer(AudioLayer *ebene) const override {
        *ebene = AudioDeviceModule::kLinuxPulseAudio;
        return 0;
    }

    int32_t RegisterAudioCallback(webrtc::AudioTransport *rueckruf) override {
        weiter_ = rueckruf;
        return 0;
    }

    int32_t Init() override { return 0; }
    int32_t Terminate() override {
        StopRecording();
        StopPlayout();
        return 0;
    }
    bool Initialized() const override { return true; }

    int16_t PlayoutDevices() override { return 1; }
    int16_t RecordingDevices() override { return 1; }

    int32_t InitPlayout() override { return 0; }
    bool PlayoutIsInitialized() const override { return true; }
    int32_t InitRecording() override { return 0; }
    bool RecordingIsInitialized() const override { return true; }

    int32_t StartPlayout() override {
        if (spielt_) {
            return 0;
        }
        spielt_ = true;
        wiedergabe_ = std::thread([this] { wiedergabeschleife(); });
        return 0;
    }

    int32_t StopPlayout() override {
        if (!spielt_) {
            return 0;
        }
        spielt_ = false;
        if (wiedergabe_.joinable()) {
            wiedergabe_.join();
        }
        return 0;
    }

    bool Playing() const override { return spielt_; }

    int32_t StartRecording() override {
        if (nimmt_auf_) {
            return 0;
        }
        nimmt_auf_ = true;
        aufnahme_ = std::thread([this] { aufnahmeschleife(); });
        return 0;
    }

    int32_t StopRecording() override {
        if (!nimmt_auf_) {
            return 0;
        }
        nimmt_auf_ = false;
        if (aufnahme_.joinable()) {
            aufnahme_.join();
        }
        return 0;
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

    /// Was zuletzt schiefging -- der Tonprozess schreibt es ins Protokoll.
    std::string letzterFehler() const override { return fehler_; }

private:
    /// Dem Tonserver sagen, wozu der Strom da ist.
    ///
    /// Ohne diese Angabe landet alles auf sink.music -- der
    /// Lautsprecher. Nokias Politikschicht routet einen Strom mit
    /// media.role=phone auf den Hoerer, so wie sie es fuer die
    /// Telefon-App tut. pa_simple nimmt keine Eigenschaftsliste
    /// entgegen; libpulse liest sie aber aus der Umgebung, und das ist
    /// hier der kurze Weg.
    static void rolleSetzen() {
        static bool getan = false;
        if (!getan) {
            setenv("PULSE_PROP", "media.role=phone", 1);
            getan = true;
        }
    }

    /// Den Tonfaden vordraengeln.
    ///
    /// PulseAudio laeuft mit nice -11 und Echtzeitprioritaet 5; unsere
    /// Faeden liefen mit der Vorgabe. Auf einem Kern, auf dem daneben
    /// kodiert wird, sind das die 57-ms-Ausreisser im Aufnahmetakt --
    /// und ein ungleichmaessiger Takt laesst den Jitterpuffer der
    /// Gegenstelle wachsen. Das hoert sie als Verzoegerung.
    ///
    /// Geht Echtzeit nicht (die Rechte dafuer haben wir vielleicht
    /// nicht), bleibt nice. Beides zu versuchen kostet nichts; was
    /// davon geklappt hat, steht danach im Protokoll.
    static void vordraengeln(const char *wer) {
        sched_param p{};
        p.sched_priority = 4;   // eins unter PulseAudio
        int fehler = pthread_setschedparam(pthread_self(), SCHED_FIFO, &p);
        if (fehler == 0) {
            std::fprintf(stderr, "[ton] %s laeuft in Echtzeit\n", wer);
            std::fflush(stderr);
            return;
        }
        errno = 0;
        if (setpriority(PRIO_PROCESS, 0, -10) == 0) {
            std::fprintf(stderr, "[ton] %s auf nice -10 (Echtzeit: %s)\n",
                         wer, std::strerror(fehler));
        } else {
            std::fprintf(stderr, "[ton] %s bleibt gewoehnlich (Echtzeit: %s)\n",
                         wer, std::strerror(fehler));
        }
        std::fflush(stderr);
    }

    static pa_sample_spec form() {
        pa_sample_spec s{};
        s.format = PA_SAMPLE_S16LE;
        s.rate = kRate;
        s.channels = kKanaele;
        return s;
    }

    // Zwei Puffer je 10 ms. Groesser waere traeger, kleiner brauchte
    // mehr Aufwachvorgaenge -- und Aufwachen ist auf einem Kern, der
    // daneben kodiert, das Teure.
    static pa_buffer_attr puffer(bool aufnahme) {
        pa_buffer_attr a{};
        const uint32_t rahmen = kSamplesJeRahmen * 2; // Bytes
        a.maxlength = rahmen * 4;
        a.fragsize = rahmen;                 // gilt fuer die Aufnahme
        a.tlength = rahmen * 2;              // gilt fuer die Wiedergabe
        a.prebuf = rahmen;
        a.minreq = rahmen;
        (void)aufnahme;
        return a;
    }

    void aufnahmeschleife() {
        rolleSetzen();
        vordraengeln("Aufnahme");
        int fehler = 0;
        auto spez = form();
        auto attr = puffer(true);
        pa_simple *s = pa_simple_new(nullptr, "drahtpost-ton", PA_STREAM_RECORD,
                                     quelle_.empty() ? nullptr : quelle_.c_str(),
                                     "Anruf", &spez, nullptr, &attr, &fehler);
        if (!s) {
            fehler_ = std::string("Aufnahme: ") + pa_strerror(fehler);
            nimmt_auf_ = false;
            return;
        }
        std::vector<int16_t> rahmen(kSamplesJeRahmen * 2);
        while (nimmt_auf_) {
            if (pa_simple_read(s, rahmen.data(), kSamplesJeRahmen * 2, &fehler) < 0) {
                fehler_ = std::string("Lesen: ") + pa_strerror(fehler);
                break;
            }
            if (stumm_) {
                std::memset(rahmen.data(), 0, kSamplesJeRahmen * 2);
            }
            auto *w = weiter_.load();
            if (!w) {
                continue;
            }
            // Die Verzoegerung, die wir melden, ist die des Puffers --
            // AEC und der Jitterpuffer rechnen damit. Geraten waere hier
            // schlechter als gemessen.
            pa_usec_t verzug = pa_simple_get_latency(s, &fehler);
            takt_melden(verzug);
            uint32_t neu = 0;
            // WebRTC will die GESAMTE Verzoegerung, nicht nur die der
            // Aufnahme: Aufnahme plus Wiedergabe. Nur die halbe zu
            // melden heisst, dass der Jitterpuffer der Gegenstelle
            // falsch rechnet.
            uint32_t ganz = (uint32_t)(verzug / 1000) + wiedergabeverzug_.load();
            w->RecordedDataIsAvailable(rahmen.data(), kSamplesJeRahmen, 2, kKanaele,
                                       kRate, ganz, 0, 0, false, neu);
        }
        pa_simple_free(s);
    }

    void wiedergabeschleife() {
        rolleSetzen();
        vordraengeln("Wiedergabe");
        int fehler = 0;
        auto spez = form();
        auto attr = puffer(false);
        pa_simple *s = pa_simple_new(nullptr, "drahtpost-ton", PA_STREAM_PLAYBACK,
                                     senke_.empty() ? nullptr : senke_.c_str(),
                                     "Anruf", &spez, nullptr, &attr, &fehler);
        if (!s) {
            fehler_ = std::string("Wiedergabe: ") + pa_strerror(fehler);
            spielt_ = false;
            return;
        }
        // Doppelt so gross wie noetig: NeedMorePlayData bekommt die
        // Kanalzahl gesagt, aber ein Puffer, der genau passt, verzeiht
        // keinen Irrtum -- und ein Ueberlauf hier zerlegt den Haufen an
        // einer Stelle, die nichts mehr mit dem Ton zu tun hat.
        int seit_messung = 0;
        std::vector<int16_t> rahmen(kSamplesJeRahmen * 2);
        while (spielt_) {
            size_t heraus = 0;
            int64_t vergangen = 0, ntp = 0;
            auto *w = weiter_.load();
            if (w) {
                w->NeedMorePlayData(kSamplesJeRahmen, 2, kKanaele, kRate,
                                    rahmen.data(), heraus, &vergangen, &ntp);
            }
            // Gibt WebRTC nichts her, wird Stille geschrieben -- nicht
            // gewartet. Sonst bliebe die Senke leer und liefe leer,
            // und der Wiederanlauf knackt.
            if (heraus != (size_t)kSamplesJeRahmen) {
                std::memset(rahmen.data(), 0, kSamplesJeRahmen * 2);
            }
            // Alle hundert Rahmen nachsehen, wie tief die Senke steht --
            // oefter waere ein Rundgang zum Tonserver je Rahmen.
            if (++seit_messung >= 100) {
                seit_messung = 0;
                pa_usec_t v = pa_simple_get_latency(s, &fehler);
                wiedergabeverzug_.store((uint32_t)(v / 1000));
            }
            if (pa_simple_write(s, rahmen.data(), kSamplesJeRahmen * 2, &fehler) < 0) {
                fehler_ = std::string("Schreiben: ") + pa_strerror(fehler);
                break;
            }
        }
        pa_simple_drain(s, &fehler);
        pa_simple_free(s);
    }

    /// Alle fuenf Sekunden eine Zeile ueber den Aufnahmetakt.
    ///
    /// Der Gegenstelle klang es verzoegert, und geraten haben wir
    /// genug: was zaehlt, ist ob die Rahmen gleichmaessig kommen. Ein
    /// Mittelwert von 10 ms bei grosser Streuung laesst den
    /// Jitterpuffer der Gegenstelle wachsen -- und genau das hoert man
    /// als Verzoegerung.
    void takt_melden(pa_usec_t verzug) {
        auto jetzt = std::chrono::steady_clock::now();
        if (n_ > 0) {
            double ms = std::chrono::duration<double, std::milli>(jetzt - letzte_).count();
            summe_ += ms;
            if (ms > gross_) gross_ = ms;
            if (ms < klein_) klein_ = ms;
        }
        letzte_ = jetzt;
        ++n_;
        if (n_ >= 500) {
            std::fprintf(stderr,
                         "[ton] Aufnahme: %.2f ms im Mittel (%.2f bis %.2f), Verzug %lu ms\n",
                         summe_ / (n_ - 1), klein_, gross_,
                         (unsigned long)(verzug / 1000));
            std::fflush(stderr);
            n_ = 0;
            summe_ = 0;
            gross_ = 0;
            klein_ = 1e9;
        }
    }

    int n_ = 0;
    double summe_ = 0, gross_ = 0, klein_ = 1e9;
    std::chrono::steady_clock::time_point letzte_;

    const std::string quelle_;
    const std::string senke_;
    std::atomic<webrtc::AudioTransport *> weiter_{nullptr};
    std::atomic<bool> spielt_{false};
    std::atomic<bool> nimmt_auf_{false};
    std::atomic<bool> stumm_{false};
    std::atomic<uint32_t> wiedergabeverzug_{0};
    std::string fehler_;
    std::thread wiedergabe_;
    std::thread aufnahme_;
};

inline rtc::scoped_refptr<PulsGeraet> neuesGeraet(const std::string &quelle,
                                                     const std::string &senke) {
    return rtc::make_ref_counted<PulsGeraet>(quelle, senke);
}

} // namespace drahtpost
