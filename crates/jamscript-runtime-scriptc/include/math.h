#ifndef JAMSCRIPT_SCRIPTC_MATH_H
#define JAMSCRIPT_SCRIPTC_MATH_H
double fmod(double, double);
double floor(double);
double trunc(double);
double ldexp(double, int);
double exp2(double);
double fabs(double);
int isnan(double);
int isinf(double);
int isfinite(double);
int signbit(double);
#define NAN (__builtin_nanf("") + 0.0)
#define INFINITY (__builtin_huge_val())
#endif
