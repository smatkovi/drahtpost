/* sys/auxv.h fuer glibc 2.10 (kam erst mit 2.16).
 *
 * getauxval liest die Hilfstabelle, die der Kern jedem Prozess beim
 * Start mitgibt -- unter anderem die Faehigkeiten der CPU. Sie steht
 * auch in /proc/self/auxv, als Folge von Paaren (Typ, Wert). Genau das
 * liest diese Fassung.
 *
 * Auf einem neueren System nimmt include_next das Original.
 */
#ifndef HARMATTAN_SYS_AUXV_H
#define HARMATTAN_SYS_AUXV_H

#include <features.h>

#if defined(__GLIBC__) && defined(__GLIBC_PREREQ) && __GLIBC_PREREQ(2, 16)
#include_next <sys/auxv.h>
#else

#include <stdio.h>

#ifndef AT_PLATFORM
#define AT_PLATFORM 15
#endif
#ifndef AT_HWCAP
#define AT_HWCAP 16
#endif
#ifndef AT_HWCAP2
#define AT_HWCAP2 26
#endif
#ifndef HWCAP_NEON
#define HWCAP_NEON (1 << 12)
#endif
#ifndef HWCAP_VFPv3
#define HWCAP_VFPv3 (1 << 13)
#endif

#ifdef __cplusplus
extern "C" {
#endif

static inline unsigned long getauxval(unsigned long art) {
  FILE* f = fopen("/proc/self/auxv", "rb");
  if (!f) return 0;
  unsigned long paar[2];
  unsigned long wert = 0;
  while (fread(paar, sizeof(paar), 1, f) == 1) {
    if (paar[0] == 0) break;
    if (paar[0] == art) { wert = paar[1]; break; }
  }
  fclose(f);
  return wert;
}

#ifdef __cplusplus
}
#endif

#endif
#endif
