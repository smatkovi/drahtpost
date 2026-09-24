/* Was glibc 2.10 von 2009 noch nicht hatte.
 *
 * pthread_setname_np kam erst mit 2.12. Es benennt nur Faeden fuer den
 * Debugger; ohne es laeuft alles, die Faeden heissen bloss nicht.
 */
#ifndef TGVOIP_KOMPAT_H
#define TGVOIP_KOMPAT_H
#include <pthread.h>
#ifndef pthread_setname_np
#define pthread_setname_np(faden, name) (0)
#endif
#endif
