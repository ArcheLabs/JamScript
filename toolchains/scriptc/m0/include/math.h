#ifndef JAMSCRIPT_M0_MATH_H
#define JAMSCRIPT_M0_MATH_H

double fmod(double left, double right);
double floor(double value);
double trunc(double value);
double ldexp(double value, int exponent);
double exp2(double value);
double fabs(double value);
int isnan(double value);
int isinf(double value);
int isfinite(double value);
int signbit(double value);

#define NAN (__builtin_nanf("") + 0.0)
#define INFINITY (__builtin_huge_val())

#endif
