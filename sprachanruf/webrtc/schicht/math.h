/* math.h fuer Harmattan: das echte, plus die fehlende
   long-double-Mathematik.
   
   Sie steht hier und nicht im Zwangs-Include, damit sie nur dort
   auftaucht, wo math.h ohnehin gebraucht wird -- bindgen (Rust) zieht
   sich sonst C++-Kopfdateien in den Parser und scheitert daran. */
#ifndef HARMATTAN_MATH_H
#define HARMATTAN_MATH_H
#include_next <math.h>
#ifdef __cplusplus
/* Die long-double-Mathematik fehlt.
 *
 * Auf ARM setzt glibc __NO_LONG_DOUBLE_MATH, und math.h erklaert die
 * ...l-Funktionen dann gar nicht (bits/mathcalls.h wird fuer long double
 * schlicht nicht durchlaufen). libc++ bindet sie mit "using ::atan2l"
 * ein und scheitert daran. Da long double auf dieser Plattform ohnehin
 * genau double ist, sind diese Weiterleitungen exakt und nicht bloss
 * eine Naeherung. */
/* math.h ZUERST: erst dadurch entsteht __NO_LONG_DOUBLE_MATH, und
   vorher danach zu fragen hiesse, Henne und Ei zu vertauschen. */
#include <math.h>
#if defined(__NO_LONG_DOUBLE_MATH) && !defined(HARMATTAN_LANGE_MATHEMATIK)
#define HARMATTAN_LANGE_MATHEMATIK 1
static inline long double acosl(long double x) { return acos((double)x); }
static inline long double asinl(long double x) { return asin((double)x); }
static inline long double atanl(long double x) { return atan((double)x); }
static inline long double cosl(long double x) { return cos((double)x); }
static inline long double sinl(long double x) { return sin((double)x); }
static inline long double tanl(long double x) { return tan((double)x); }
static inline long double acoshl(long double x) { return acosh((double)x); }
static inline long double asinhl(long double x) { return asinh((double)x); }
static inline long double atanhl(long double x) { return atanh((double)x); }
static inline long double coshl(long double x) { return cosh((double)x); }
static inline long double sinhl(long double x) { return sinh((double)x); }
static inline long double tanhl(long double x) { return tanh((double)x); }
static inline long double expl(long double x) { return exp((double)x); }
static inline long double exp2l(long double x) { return exp2((double)x); }
static inline long double expm1l(long double x) { return expm1((double)x); }
static inline long double logl(long double x) { return log((double)x); }
static inline long double log10l(long double x) { return log10((double)x); }
static inline long double log1pl(long double x) { return log1p((double)x); }
static inline long double log2l(long double x) { return log2((double)x); }
static inline long double logbl(long double x) { return logb((double)x); }
static inline long double sqrtl(long double x) { return sqrt((double)x); }
static inline long double cbrtl(long double x) { return cbrt((double)x); }
static inline long double fabsl(long double x) { return fabs((double)x); }
static inline long double ceill(long double x) { return ceil((double)x); }
static inline long double floorl(long double x) { return floor((double)x); }
static inline long double rintl(long double x) { return rint((double)x); }
static inline long double roundl(long double x) { return round((double)x); }
static inline long double truncl(long double x) { return trunc((double)x); }
static inline long double nearbyintl(long double x) { return nearbyint((double)x); }
static inline long double erfl(long double x) { return erf((double)x); }
static inline long double erfcl(long double x) { return erfc((double)x); }
static inline long double lgammal(long double x) { return lgamma((double)x); }
static inline long double tgammal(long double x) { return tgamma((double)x); }
static inline long double atan2l(long double x, long double y) { return atan2((double)x, (double)y); }
static inline long double powl(long double x, long double y) { return pow((double)x, (double)y); }
static inline long double fmodl(long double x, long double y) { return fmod((double)x, (double)y); }
static inline long double hypotl(long double x, long double y) { return hypot((double)x, (double)y); }
static inline long double copysignl(long double x, long double y) { return copysign((double)x, (double)y); }
static inline long double fdiml(long double x, long double y) { return fdim((double)x, (double)y); }
static inline long double fmaxl(long double x, long double y) { return fmax((double)x, (double)y); }
static inline long double fminl(long double x, long double y) { return fmin((double)x, (double)y); }
static inline long double nextafterl(long double x, long double y) { return nextafter((double)x, (double)y); }
static inline long double remainderl(long double x, long double y) { return remainder((double)x, (double)y); }
static inline long double fmal(long double a, long double b, long double c) { return fma((double)a,(double)b,(double)c); }
static inline long double ldexpl(long double x, int e) { return ldexp((double)x, e); }
static inline long double frexpl(long double x, int* e) { return frexp((double)x, e); }
static inline long double scalbnl(long double x, int e) { return scalbn((double)x, e); }
static inline long double modfl(long double x, long double* p) { double d; double r = modf((double)x, &d); *p = d; return r; }
static inline long double remquol(long double x, long double y, int* q) { return remquo((double)x,(double)y,q); }
static inline int ilogbl(long double x) { return ilogb((double)x); }
static inline long lrintl(long double x) { return lrint((double)x); }
static inline long lroundl(long double x) { return lround((double)x); }
static inline long long llrintl(long double x) { return llrint((double)x); }
static inline long long llroundl(long double x) { return llround((double)x); }
static inline long double nanl(const char* t) { return nan(t); }
#endif

#endif
#endif
