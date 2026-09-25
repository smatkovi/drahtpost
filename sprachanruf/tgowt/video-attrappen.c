/* Attrappen fuer H.264 -- openh264 und ffmpeg.
 *
 * tg_owt verweist aus seinen Codec-Fabriken unbedingt auf beide, auch
 * wenn kein H.264 vorkommt. Auf Harmattan bauen wir sie nicht: openh264
 * und Chromiums ffmpeg-Abstimmung sind zwei weitere Kreuzbauten fuer
 * einen Codec, den ein OMAP3630 in Software ohnehin nicht in Echtzeit
 * schafft. VP8 dagegen ist echt -- libvpx ist mitgebaut.
 *
 * Alle Attrappen scheitern so, wie es die Schnittstelle vorsieht: der
 * Aufrufer bekommt einen Fehlerwert oder einen Nullzeiger und meldet
 * H.264 dann einfach nicht als Faehigkeit an. Keine davon luegt.
 */
#include <stddef.h>

/* openh264 */
int WelsCreateSVCEncoder(void **encoder) {
    if (encoder) {
        *encoder = NULL;
    }
    return 1; /* cmInitParaError: alles ausser 0 heisst gescheitert */
}
void WelsDestroySVCEncoder(void *encoder) { (void)encoder; }

/* ffmpeg: libavcodec und libavutil, nur der Dekodierpfad von tg_owt */
void *av_buffer_create(unsigned char *d, size_t s, void (*f)(void *, unsigned char *), void *o, int fl) {
    (void)d; (void)s; (void)f; (void)o; (void)fl; return NULL;
}
void *av_buffer_get_opaque(const void *b) { (void)b; return NULL; }
void *av_frame_alloc(void) { return NULL; }
void av_frame_free(void **f) { (void)f; }
void av_frame_unref(void *f) { (void)f; }
int av_image_check_size(unsigned w, unsigned h, int fl, void *l) {
    (void)w; (void)h; (void)fl; (void)l; return -22; /* AVERROR(EINVAL) */
}
void *av_packet_alloc(void) { return NULL; }
void av_packet_free(void **p) { (void)p; }
void avcodec_align_dimensions(void *c, int *w, int *h) { (void)c; (void)w; (void)h; }
void *avcodec_alloc_context3(const void *c) { (void)c; return NULL; }
const void *avcodec_find_decoder(int id) { (void)id; return NULL; }
void avcodec_free_context(void **c) { (void)c; }
int avcodec_open2(void *c, const void *codec, void **opts) {
    (void)c; (void)codec; (void)opts; return -22;
}
int avcodec_receive_frame(void *c, void *f) { (void)c; (void)f; return -22; }
int avcodec_send_packet(void *c, const void *p) { (void)c; (void)p; return -22; }
