/* Was Kernel 2.6.32 und glibc 2.10 von 2009 noch nicht kannten.
 *
 * Wird ueber build/config/compiler/BUILD.gn in JEDE Uebersetzung
 * gezogen. Die Wachter sorgen dafuer, dass auf dem Baurechner (modernes
 * glibc) nichts davon greift -- dort gibt es alles laengst, und eine
 * zweite Erklaerung waere ein Fehler.
 */
#ifndef HARMATTAN_SCHICHT_H
#define HARMATTAN_SCHICHT_H
/* Die Formatmakros (PRIu64, PRIX32 …) versteckt das alte glibc in C++,
   wenn dieses Zeichen nicht VOR dem ersten Einbinden steht. Deshalb hier
   ganz oben, vor allem anderen. */
#ifndef __STDC_FORMAT_MACROS
#define __STDC_FORMAT_MACROS 1
#endif
#ifndef __STDC_CONSTANT_MACROS
#define __STDC_CONSTANT_MACROS 1
#endif
#ifndef __STDC_LIMIT_MACROS
#define __STDC_LIMIT_MACROS 1
#endif
/* static_assert kam mit C11; die assert.h von 2009 kennt es nicht.
   _Static_assert ist ein Schluesselwort des Uebersetzers und immer da. */
#if !defined(__cplusplus) && !defined(static_assert)
#define static_assert _Static_assert
#endif

#ifdef __cplusplus

/* Uhren. Die Zahlen sind die des Linux-Kerns; fehlt eine wirklich,
   liefert clock_gettime EINVAL, und der aufrufende Code faellt von
   selbst zurueck. */
#ifndef CLOCK_MONOTONIC_RAW
#define CLOCK_MONOTONIC_RAW 4
#endif
#ifndef CLOCK_REALTIME_COARSE
#define CLOCK_REALTIME_COARSE 5
#endif
#ifndef CLOCK_MONOTONIC_COARSE
#define CLOCK_MONOTONIC_COARSE 6
#endif
#ifndef CLOCK_BOOTTIME
#define CLOCK_BOOTTIME 7
#endif


/* BoringSSL fragt ueber sys/auxv.h zur Laufzeit, ob die CPU NEON hat --
   diese Kopfdatei gibt es erst ab glibc 2.16. Wir wissen es aber schon:
   der OMAP3630 im N950 hat NEON. Mit OPENSSL_STATIC_ARMCAP faellt die
   ganze Erkennung weg. */
#if defined(__arm__) && !defined(OPENSSL_STATIC_ARMCAP)
#define OPENSSL_STATIC_ARMCAP 1
#define OPENSSL_STATIC_ARMCAP_NEON 1
#endif


/* getrandom kam mit Kernel 3.17 (2014); unserer ist 2.6.32. Die Zahlen
   hier erlauben das Uebersetzen; der Aufruf scheitert dann zur Laufzeit
   mit ENOSYS, und BoringSSL faellt selbst auf /dev/urandom zurueck --
   genau dafuer ist der Rueckfall dort vorgesehen. */
#if defined(__arm__)
#ifndef __NR_getrandom
#define __NR_getrandom 384
#endif
#endif
#ifndef GRND_NONBLOCK
#define GRND_NONBLOCK 0x0001
#endif
#ifndef GRND_RANDOM
#define GRND_RANDOM 0x0002
#endif


/* TCP_USER_TIMEOUT kam mit Kernel 2.6.37; unserer ist 2.6.32. setsockopt
   lehnt die Option dann zur Laufzeit mit ENOPROTOOPT ab -- das verkraftet
   der Aufrufer, er protokolliert es hoechstens. */
#ifndef TCP_USER_TIMEOUT
#define TCP_USER_TIMEOUT 18
#endif


#endif /* __cplusplus */
#endif
