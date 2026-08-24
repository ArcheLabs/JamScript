#include <stdarg.h>
#include <stddef.h>

typedef struct JamscriptM1File FILE;
FILE *stdout = (FILE *)0;
FILE *stderr = (FILE *)0;

int vsnprintf(char *buffer, size_t size, const char *format, va_list args) {
  (void)buffer; (void)size; (void)format; (void)args;
  return -1;
}

int snprintf(char *buffer, size_t size, const char *format, ...) {
  (void)buffer; (void)size; (void)format;
  return -1;
}
