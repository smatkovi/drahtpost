/* Was kostet Opus auf der N950?
 *
 * Telegrams altes Protokoll schickt Opus in 20-ms-Rahmen bei 48 kHz.
 * Ein Rahmen muss also in 20 ms fertig sein -- Kodieren UND Dekodieren
 * zusammen, denn beides laeuft im Gespraech gleichzeitig.
 */
#include <opus.h>
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
#include <time.h>

static double jetzt(void) {
	struct timespec t;
	clock_gettime(CLOCK_MONOTONIC, &t);
	return t.tv_sec + t.tv_nsec / 1e9;
}

static void messen(int rate, int komplexitaet, int bitrate) {
	int fehler = 0;
	OpusEncoder *enc = opus_encoder_create(rate, 1, OPUS_APPLICATION_VOIP, &fehler);
	if (fehler != OPUS_OK) { printf("Kodierer: %s\n", opus_strerror(fehler)); return; }
	OpusDecoder *dec = opus_decoder_create(rate, 1, &fehler);
	if (fehler != OPUS_OK) { printf("Dekodierer: %s\n", opus_strerror(fehler)); return; }
	opus_encoder_ctl(enc, OPUS_SET_COMPLEXITY(komplexitaet));
	opus_encoder_ctl(enc, OPUS_SET_BITRATE(bitrate));
	opus_encoder_ctl(enc, OPUS_SET_SIGNAL(OPUS_SIGNAL_VOICE));

	const int rahmen = rate / 50;           /* 20 ms */
	opus_int16 *pcm = malloc(rahmen * sizeof(opus_int16));
	opus_int16 *raus = malloc(rahmen * sizeof(opus_int16));
	unsigned char paket[1500];

	/* sprachaehnlich: Silbenhuellkurve ueber Rauschen plus Grundton */
	for (int i = 0; i < rahmen; i++) {
		double h = 0.5 * (1 - cos(2 * M_PI * i / rahmen));
		pcm[i] = (opus_int16)(h * (8000 * sin(i * 0.05) + 3000 * ((rand() % 200) - 100) / 100.0));
	}

	for (int i = 0; i < 20; i++) { int n = opus_encode(enc, pcm, rahmen, paket, sizeof paket); opus_decode(dec, paket, n, raus, rahmen, 0); }

	const int runden = 100;
	double t0 = jetzt();
	int bytes = 0, n = 0;
	for (int i = 0; i < runden; i++) { n = opus_encode(enc, pcm, rahmen, paket, sizeof paket); bytes += n; }
	double tenc = (jetzt() - t0) / runden * 1000;

	t0 = jetzt();
	for (int i = 0; i < runden; i++) opus_decode(dec, paket, n, raus, rahmen, 0);
	double tdec = (jetzt() - t0) / runden * 1000;

	printf("%5d Hz, Komplexitaet %2d, %5d bit/s: kodieren %5.2f ms, dekodieren %5.2f ms, zusammen %5.2f von 20 (%3.0f %%), %d Byte je Rahmen\n",
	       rate, komplexitaet, bitrate, tenc, tdec, tenc + tdec, (tenc + tdec) / 20 * 100, bytes / runden);

	free(pcm); free(raus);
	opus_encoder_destroy(enc); opus_decoder_destroy(dec);
}

int main(void) {
	messen(48000, 10, 24000);
	messen(48000, 5, 24000);
	messen(48000, 0, 24000);
	messen(16000, 5, 20000);
	return 0;
}
