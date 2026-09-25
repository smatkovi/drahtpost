// Was kostet Video auf einem Cortex-A8 von 2010?
//
// Dieselbe Frage wie bei AEC3, und aus demselben Grund vor dem Bauen
// gestellt: ein Videoknopf, der drei Bilder in der Sekunde liefert, waere
// schlimmer als keiner. Gemessen wird VP8 -- H.264 gibt es hier nicht,
// und der SGX530 kann ohnehin nur zeichnen, nicht kodieren.
//
// Der Massstab ist die Echtzeit. Bei 15 Bildern je Sekunde hat ein Bild
// 66,7 ms; alles darueber heisst, dass die Bildrate faellt. Und der Kern
// muss daneben noch Opus, PulseAudio und die Signalisierung tragen.

#include <chrono>
#include <cmath>
#include <cstdio>
#include <memory>
#include <vector>

#include <api/video/i420_buffer.h>
#include <api/video/video_frame.h>
#include <api/video_codecs/video_codec.h>
#include <api/video_codecs/video_encoder.h>
#include <api/environment/environment_factory.h>
#include <modules/video_coding/codecs/vp8/include/vp8.h>

namespace {

using Uhr = std::chrono::steady_clock;

/// Faengt die kodierten Bilder auf und zaehlt sie.
class Auffang : public webrtc::EncodedImageCallback {
public:
    Result OnEncodedImage(const webrtc::EncodedImage &bild,
                          const webrtc::CodecSpecificInfo *) override {
        ++bilder;
        bytes += bild.size();
        return Result(Result::OK);
    }
    int bilder = 0;
    size_t bytes = 0;
};

/// Ein Bild mit Inhalt: eine wandernde Kante und ein Verlauf.
///
/// Eine leere Flaeche waere geschoent -- VP8 wird damit so schnell
/// fertig, dass die Messung nichts mehr ueber ein Gespraech aussagt, in
/// dem sich jemand bewegt.
webrtc::scoped_refptr<webrtc::I420Buffer> bild_bauen(int w, int h, int n) {
    auto b = webrtc::I420Buffer::Create(w, h);
    uint8_t *y = b->MutableDataY();
    for (int zeile = 0; zeile < h; ++zeile) {
        for (int spalte = 0; spalte < w; ++spalte) {
            int kante = (spalte + n * 4) % w;
            y[zeile * b->StrideY() + spalte] =
                (uint8_t)(kante < w / 2 ? 40 + zeile % 60 : 200 - spalte % 50);
        }
    }
    for (int zeile = 0; zeile < (h + 1) / 2; ++zeile) {
        for (int spalte = 0; spalte < (w + 1) / 2; ++spalte) {
            b->MutableDataU()[zeile * b->StrideU() + spalte] = (uint8_t)(110 + (n % 20));
            b->MutableDataV()[zeile * b->StrideV() + spalte] = (uint8_t)(130 - (n % 20));
        }
    }
    return b;
}

void messen(int w, int h, int bitrate_kbps, int bildrate) {
    auto enc = webrtc::VP8Encoder::Create();
    webrtc::VideoCodec kodek{};
    kodek.codecType = webrtc::kVideoCodecVP8;
    kodek.width = (uint16_t)w;
    kodek.height = (uint16_t)h;
    kodek.startBitrate = (unsigned int)bitrate_kbps;
    kodek.maxBitrate = (unsigned int)bitrate_kbps * 2;
    kodek.minBitrate = (unsigned int)bitrate_kbps / 2;
    kodek.maxFramerate = (uint32_t)bildrate;
    kodek.active = true;
    kodek.qpMax = 56;
    kodek.numberOfSimulcastStreams = 0;
    *kodek.VP8() = webrtc::VideoEncoder::GetDefaultVp8Settings();
    kodek.VP8()->numberOfTemporalLayers = 1;
    kodek.VP8()->denoisingOn = false;   // kostet und hilft hier nichts
    kodek.VP8()->automaticResizeOn = true;
    // Ein Kern, und der ist auch noch geteilt.
    kodek.SetVideoEncoderComplexity(webrtc::VideoCodecComplexity::kComplexityLow);

    webrtc::VideoEncoder::Settings einst(webrtc::VideoEncoder::Capabilities(false), 1, 1200);
    if (enc->InitEncode(&kodek, einst) != WEBRTC_VIDEO_CODEC_OK) {
        std::printf("%dx%d: laesst sich nicht einrichten\n", w, h);
        return;
    }
    Auffang auffang;
    enc->RegisterEncodeCompleteCallback(&auffang);
    webrtc::VideoEncoder::RateControlParameters raten;
    raten.bitrate.SetBitrate(0, 0, (uint32_t)bitrate_kbps * 1000);
    raten.framerate_fps = bildrate;
    enc->SetRates(raten);

    const int anlauf = 5, zaehlt = 40;
    std::vector<webrtc::VideoFrameType> arten{webrtc::VideoFrameType::kVideoFrameDelta};
    double summe = 0, groesster = 0;
    for (int i = 0; i < anlauf + zaehlt; ++i) {
        auto puffer = bild_bauen(w, h, i);
        auto rahmen = webrtc::VideoFrame::Builder()
                          .set_video_frame_buffer(puffer)
                          .set_timestamp_rtp((uint32_t)(i * 90000 / bildrate))
                          .set_timestamp_us(i * 1000000 / bildrate)
                          .build();
        arten[0] = (i == 0) ? webrtc::VideoFrameType::kVideoFrameKey
                            : webrtc::VideoFrameType::kVideoFrameDelta;
        auto beginn = Uhr::now();
        enc->Encode(rahmen, &arten);
        double ms = std::chrono::duration<double, std::milli>(Uhr::now() - beginn).count();
        if (i >= anlauf) {
            summe += ms;
            if (ms > groesster) {
                groesster = ms;
            }
        }
    }
    const double je = summe / zaehlt;
    const double budget = 1000.0 / bildrate;
    std::printf("VP8 %3dx%-3d %4d kbit/s @%2d B/s  %6.1f ms je Bild (max %6.1f)  = %5.0f %% eines Kerns  ->  %4.1f B/s moeglich\n",
                w, h, bitrate_kbps, bildrate, je, groesster, 100.0 * je / budget,
                1000.0 / je);
    enc->Release();
}

/// Und die Gegenrichtung: was kostet es, das Bild der Gegenstelle zu
/// dekodieren? Das ist die Haelfte, die auch dann anfaellt, wenn wir
/// selbst kein Bild senden.
class Fangdekoder : public webrtc::DecodedImageCallback {
public:
    int32_t Decoded(webrtc::VideoFrame &) override {
        ++bilder;
        return 0;
    }
    int bilder = 0;
};

/// Sammelt die kodierten Bilder, damit sie danach dekodiert werden
/// koennen -- getrennt gemessen, sonst stuende beides in einer Zahl.
class Sammler : public webrtc::EncodedImageCallback {
public:
    Result OnEncodedImage(const webrtc::EncodedImage &bild,
                          const webrtc::CodecSpecificInfo *) override {
        stuecke.push_back(bild);
        return Result(Result::OK);
    }
    std::vector<webrtc::EncodedImage> stuecke;
};

void dekodieren_messen(int w, int h, int bitrate_kbps, int bildrate) {
    auto enc = webrtc::VP8Encoder::Create();
    webrtc::VideoCodec kodek{};
    kodek.codecType = webrtc::kVideoCodecVP8;
    kodek.width = (uint16_t)w;
    kodek.height = (uint16_t)h;
    kodek.startBitrate = (unsigned int)bitrate_kbps;
    kodek.maxBitrate = (unsigned int)bitrate_kbps * 2;
    kodek.minBitrate = (unsigned int)bitrate_kbps / 2;
    kodek.maxFramerate = (uint32_t)bildrate;
    kodek.active = true;
    kodek.qpMax = 56;
    *kodek.VP8() = webrtc::VideoEncoder::GetDefaultVp8Settings();
    kodek.VP8()->numberOfTemporalLayers = 1;
    webrtc::VideoEncoder::Settings einst(webrtc::VideoEncoder::Capabilities(false), 1, 1200);
    if (enc->InitEncode(&kodek, einst) != WEBRTC_VIDEO_CODEC_OK) {
        return;
    }
    Sammler sammler;
    enc->RegisterEncodeCompleteCallback(&sammler);
    webrtc::VideoEncoder::RateControlParameters raten;
    raten.bitrate.SetBitrate(0, 0, (uint32_t)bitrate_kbps * 1000);
    raten.framerate_fps = bildrate;
    enc->SetRates(raten);
    std::vector<webrtc::VideoFrameType> arten{webrtc::VideoFrameType::kVideoFrameKey};
    for (int i = 0; i < 30; ++i) {
        auto rahmen = webrtc::VideoFrame::Builder()
                          .set_video_frame_buffer(bild_bauen(w, h, i))
                          .set_timestamp_rtp((uint32_t)(i * 90000 / bildrate))
                          .set_timestamp_us(i * 1000000 / bildrate)
                          .build();
        arten[0] = (i == 0) ? webrtc::VideoFrameType::kVideoFrameKey
                            : webrtc::VideoFrameType::kVideoFrameDelta;
        enc->Encode(rahmen, &arten);
    }
    enc->Release();

    auto dec = webrtc::VP8Decoder::Create();
    webrtc::VideoDecoder::Settings deinst;
    deinst.set_codec_type(webrtc::kVideoCodecVP8);
    deinst.set_max_render_resolution({w, h});
    deinst.set_number_of_cores(1);
    if (!dec->Configure(deinst)) {
        return;
    }
    Fangdekoder fang;
    dec->RegisterDecodeCompleteCallback(&fang);
    double summe = 0;
    int n = 0;
    for (const auto &stueck : sammler.stuecke) {
        auto beginn = Uhr::now();
        dec->Decode(stueck, 0);
        summe += std::chrono::duration<double, std::milli>(Uhr::now() - beginn).count();
        ++n;
    }
    if (n == 0) {
        return;
    }
    const double je = summe / n;
    std::printf("VP8 %3dx%-3d dekodieren                %6.1f ms je Bild                      = %5.0f %% eines Kerns\n",
                w, h, je, 100.0 * je / (1000.0 / bildrate));
    dec->Release();
}

} // namespace

int main() {
    std::printf("Massstab: bei 15 B/s hat ein Bild 66,7 ms.\n\n");
    messen(176, 144, 100, 15);   // QCIF
    messen(320, 240, 200, 15);   // QVGA
    messen(352, 288, 250, 15);   // CIF
    messen(640, 480, 500, 15);   // VGA
    std::printf("\n");
    dekodieren_messen(176, 144, 100, 15);
    dekodieren_messen(320, 240, 200, 15);
    dekodieren_messen(352, 288, 250, 15);
    dekodieren_messen(640, 480, 500, 15);
    return 0;
}
