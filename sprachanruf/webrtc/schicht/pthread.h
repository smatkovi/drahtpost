/* pthread.h fuer Harmattan: das echte, plus die Fadennamen, die es
   erst ab glibc 2.12 gibt. Sie dienen nur dem Debugger. */
#ifndef HARMATTAN_PTHREAD_H
#define HARMATTAN_PTHREAD_H
#include_next <pthread.h>
#include <features.h>
#if defined(__GLIBC__) && defined(__GLIBC_PREREQ)
#if !__GLIBC_PREREQ(2, 12)
#include <stddef.h>
static inline int pthread_setname_np(pthread_t f, const char* n) { (void)f; (void)n; return 0; }
static inline int pthread_getname_np(pthread_t f, char* n, size_t l) { (void)f; if (l) n[0] = 0; return 0; }
#endif
#endif
#endif
