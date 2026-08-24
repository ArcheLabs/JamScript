#include <assert.h>
#include <ctype.h>
#include <math.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

/* ScriptC's selected runtime is compiled freestanding.  These shims keep
 * accidental host facilities out of the service ELF; unsupported facilities
 * fail through the guest abort trap instead of silently consulting a host. */
FILE *stdout = (FILE *)0;
FILE *stderr = (FILE *)0;

extern void *jamscript_guest_malloc(size_t);
extern void *jamscript_guest_calloc(size_t, size_t);
extern void *jamscript_guest_realloc(void *, size_t);
extern void jamscript_guest_free(void *);

void *malloc(size_t size) { return jamscript_guest_malloc(size); }
void *calloc(size_t count, size_t size) { return jamscript_guest_calloc(count, size); }
void *realloc(void *pointer, size_t size) { return jamscript_guest_realloc(pointer, size); }
void free(void *pointer) { jamscript_guest_free(pointer); }

int vsnprintf(char *buffer, size_t size, const char *format, va_list args) {
  (void)buffer;
  (void)size;
  (void)format;
  (void)args;
  return -1;
}

int snprintf(char *buffer, size_t size, const char *format, ...) {
  (void)buffer;
  (void)size;
  (void)format;
  return -1;
}

int fflush(FILE *stream) { (void)stream; return 0; }
int fputs(const char *value, FILE *stream) { (void)value; (void)stream; return -1; }
int fputc(int value, FILE *stream) { (void)value; (void)stream; return -1; }
size_t fwrite(const void *value, size_t size, size_t count, FILE *stream) {
  (void)value;
  (void)size;
  (void)stream;
  return count;
}

int isalnum(int value) { return isalpha(value) || isdigit(value); }
int isalpha(int value) { return (value >= 'a' && value <= 'z') || (value >= 'A' && value <= 'Z'); }
int isdigit(int value) { return value >= '0' && value <= '9'; }
int isspace(int value) { return value == ' ' || (value >= '\t' && value <= '\r'); }
int tolower(int value) { return value >= 'A' && value <= 'Z' ? value + ('a' - 'A') : value; }
int toupper(int value) { return value >= 'a' && value <= 'z' ? value - ('a' - 'A') : value; }

char *getenv(const char *name) { (void)name; return (char *)0; }
long strtol(const char *value, char **end, int base) {
  (void)value;
  if (end) *end = (char *)value;
  (void)base;
  return 0;
}
double strtod(const char *value, char **end) {
  (void)value;
  if (end) *end = (char *)value;
  return 0.0;
}

int isnan(double value) { return __builtin_isnan(value); }
int isinf(double value) { return __builtin_isinf(value); }
int isfinite(double value) { return __builtin_isfinite(value); }
int signbit(double value) { return __builtin_signbit(value); }
double fabs(double value) { return value < 0.0 ? -value : value; }
double trunc(double value) { return value < 0.0 ? -((double)(-(long)value)) : (double)(long)value; }
double floor(double value) {
  double integer = trunc(value);
  return value < integer ? integer - 1.0 : integer;
}
double fmod(double value, double divisor) {
  if (divisor == 0.0) return 0.0;
  return value - trunc(value / divisor) * divisor;
}
double ldexp(double value, int exponent) {
  double factor = 1.0;
  int direction = exponent < 0 ? -1 : 1;
  for (int index = 0; index < (exponent < 0 ? -exponent : exponent); ++index)
    factor = direction > 0 ? factor * 2.0 : factor * 0.5;
  return value * factor;
}
double exp2(double value) { return ldexp(1.0, (int)value); }

void __stack_chk_fail(void) { abort(); }
