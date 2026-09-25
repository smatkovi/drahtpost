// Was kostet WebRTCs Tonaufbereitung auf einem Kern von 2010?
//
// Das ist die Frage, die der Bindebeweis offenlaesst. tgcalls v2 schaltet
// Echoausloeschung (AEC3) und Rauschunterdrueckung fest ein --
// _disableOutgoingAudioProcessing steht in InstanceV2Impl.cpp auf false.
// AEC3 ist der teuerste Teil von WebRTC ausserhalb der Videocodecs.
//
// Gemessen wird in Vielfachen der Echtzeit: ein 10-ms-Rahmen darf 10 ms
// kosten, dann ist der Kern genau ausgelastet. Alles ueber etwa 30 %
// eines Kerns ist auf der N950 zu viel, weil Pulseaudio, die SIP-Bruecke
// und der Anrufdienst daneben laufen muessen.

#include <cstdio>
#include <chrono>
#include <vector>
#include <cmath>

#include <modules/audio_processing/include/audio_processing.h>
#include <opus.h>

namespace {

using Uhr = std::chrono::steady_clock;

// Sprachaehnliches Rauschen: echte Nulldaten wuerden die
// Rauschunterdrueckung in einen billigen Sonderfall laufen lassen.
std::vector<int16_t> stimme(int samples) {
    std::vector<int16_t> aus(samples);
    for (int i = 0; i < samples; ++i) {
        double t = i / 48000.0;
        double w = 0.35 * std::sin(2 * M_PI * 220 * t)
                 + 0.20 * std::sin(2 * M_PI * 440 * t)
                 + 0.10 * std::sin(2 * M_PI * 1700 * t);
        aus[i] = (int16_t)(w * 12000);
    }
    return aus;
}

double messen(const char *name, int rahmen, int samplesJeRahmen,
              double rahmenMs, void (*arbeit)(int), int vorlauf = 20) {
    for (int i = 0; i < vorlauf; ++i) {
        arbeit(i);
    }
    auto beginn = Uhr::now();
    for (int i = 0; i < rahmen; ++i) {
        arbeit(i);
    }
    double gesamt = std::chrono::duration<double, std::milli>(Uhr::now() - beginn).count();
    double jeRahmen = gesamt / rahmen;
    std::printf("%-34s %7.3f ms je %g-ms-Rahmen  =  %5.1f %% eines Kerns\n",
                name, jeRahmen, rahmenMs, 100.0 * jeRahmen / rahmenMs);
    (void)samplesJeRahmen;
    return jeRahmen;
}

webrtc::AudioProcessing *apm = nullptr;
std::vector<int16_t> ton;
std::vector<int16_t> hinaus;
webrtc::StreamConfig lage(48000, 1);

void apmDurchlauf(int i) {
    const int16_t *quelle = ton.data() + (i % 50) * 480;
    // AEC3 braucht beide Seiten: ohne die Wiedergabeseite hat es nichts
    // zu vergleichen und die Messung waere geschoent.
    apm->ProcessReverseStream(quelle, lage, lage, hinaus.data());
    apm->ProcessStream(quelle, lage, lage, hinaus.data());
}

OpusEncoder *opus = nullptr;
std::vector<unsigned char> paket;

void opusDurchlauf(int i) {
    const int16_t *quelle = ton.data() + (i % 25) * 960;
    opus_encode(opus, quelle, 960, paket.data(), (opus_int32)paket.size());
}

} // namespace

int main() {
    ton = stimme(48000);
    hinaus.resize(480);
    paket.resize(4000);

    // 1. Wie tgcalls v2 es baut: AEC3 und Rauschunterdrueckung an.
    {
        webrtc::AudioProcessingBuilder bauer;
        auto gebaut = bauer.Create();
        apm = gebaut.get();
        webrtc::AudioProcessing::Config cfg;
        cfg.echo_canceller.enabled = true;
        cfg.echo_canceller.mobile_mode = false;
        cfg.noise_suppression.enabled = true;
        cfg.gain_controller1.enabled = true;
        cfg.high_pass_filter.enabled = true;
        apm->ApplyConfig(cfg);
        messen("APM: AEC3 + NS + AGC + HPF", 300, 480, 10.0, apmDurchlauf);

        // 2. AEC3 im Handy-Modus (AECM): derselbe Zweck, ein Bruchteil
        //    der Rechnung -- dafuer gebaut worden, als Telefone so
        //    schnell waren wie dieses.
        cfg.echo_canceller.mobile_mode = true;
        apm->ApplyConfig(cfg);
        messen("APM: AECM (Handy-Modus) + NS", 300, 480, 10.0, apmDurchlauf);

        // 3. Alles aus. Der Ton der N950 kommt ueber source.voice, also
        //    durch Nokias eigene Sprachaufbereitung -- WebRTCs waere
        //    dann doppelt gemoppelt.
        cfg.echo_canceller.enabled = false;
        cfg.noise_suppression.enabled = false;
        cfg.gain_controller1.enabled = false;
        cfg.high_pass_filter.enabled = false;
        apm->ApplyConfig(cfg);
        messen("APM: alles aus", 300, 480, 10.0, apmDurchlauf);
        apm = nullptr;
    }

    // 4. Opus, wie tgcalls ihn fahren wuerde: 48 kHz, 20 ms, Sprache.
    int fehler = 0;
    opus = opus_encoder_create(48000, 1, OPUS_APPLICATION_VOIP, &fehler);
    if (!opus) {
        std::printf("Opus liess sich nicht anlegen: %d\n", fehler);
        return 1;
    }
    opus_encoder_ctl(opus, OPUS_SET_BITRATE(24000));
    opus_encoder_ctl(opus, OPUS_SET_COMPLEXITY(5));
    messen("Opus 24 kbit/s, Komplexitaet 5", 200, 960, 20.0, opusDurchlauf);
    opus_encoder_ctl(opus, OPUS_SET_COMPLEXITY(0));
    messen("Opus 24 kbit/s, Komplexitaet 0", 200, 960, 20.0, opusDurchlauf);
    opus_encoder_destroy(opus);
    return 0;
}
