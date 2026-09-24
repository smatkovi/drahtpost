// Probe: bindet libtgvoip fuer Harmattan, und laeuft es dort an?
//
// Krypto ist hier absichtlich Attrappe -- geprueft wird, ob die
// Bibliothek uebersetzt, bindet und ihre Fadenmaschinerie auf einem
// Kernel von 2009 anlaeuft. Die echten Funktionen liefert spaeter der
// Daemon.
#include "VoIPController.h"
#include "audio/AudioIOCallback.h"
#include <cstdio>
#include <cstring>
#include <cstdlib>
#include <ctime>

using namespace tgvoip;

namespace tgvoip { CryptoFunctions crypto; }

static void attrappe_rand(uint8_t* b, size_t n) { for (size_t i = 0; i < n; i++) b[i] = (uint8_t)rand(); }
static void attrappe_hash(uint8_t*, size_t, uint8_t* out) { memset(out, 0, 32); }
static void attrappe_ige(uint8_t* in, uint8_t* out, size_t n, uint8_t*, uint8_t*) { memcpy(out, in, n); }
static void attrappe_ctr(uint8_t*, size_t, uint8_t*, uint8_t*, uint8_t*, uint32_t*) {}
static void attrappe_cbc(uint8_t* in, uint8_t* out, size_t n, uint8_t*, uint8_t*) { memcpy(out, in, n); }

int main() {
	crypto.rand_bytes = attrappe_rand;
	crypto.sha1 = attrappe_hash;
	crypto.sha256 = attrappe_hash;
	crypto.aes_ige_encrypt = attrappe_ige;
	crypto.aes_ige_decrypt = attrappe_ige;
	crypto.aes_ctr_encrypt = attrappe_ctr;
	crypto.aes_cbc_encrypt = attrappe_cbc;
	crypto.aes_cbc_decrypt = attrappe_cbc;

	printf("libtgvoip %s\n", VoIPController::GetVersion());
	printf("Verbindungsstufe (min/max): %d/%d\n",
	       VoIPController::GetConnectionMaxLayer(), VoIPController::GetConnectionMaxLayer());

	VoIPController* c = new VoIPController();
	printf("VoIPController angelegt\n");

	// Der Ton kaeme spaeter von der SIP-Bruecke; hier nur der Nachweis,
	// dass die Rueckruf-Tonschnittstelle vorhanden ist.
	c->SetAudioDataCallbacks(
		[](int16_t*, size_t) {},   // was wir senden: kaeme von der Bruecke
		[](int16_t*, size_t) {});  // was wir hoeren: ginge an die Bruecke
	printf("Ton-Rueckrufe gesetzt\n");

	// Stop() vor delete: die Bibliothek besteht darauf, und zwar mit
	// Nachdruck ("CALL controller->Stop() BEFORE DELETING").
	c->Stop();
	delete c;
	printf("wieder abgeraeumt -- libtgvoip laeuft auf diesem Geraet\n");
	return 0;
}
