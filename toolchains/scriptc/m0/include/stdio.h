#ifndef JAMSCRIPT_M0_STDIO_H
#define JAMSCRIPT_M0_STDIO_H

#include <stddef.h>
#include <stdarg.h>

typedef struct JamscriptM0File FILE;

int snprintf(char *buffer, size_t size, const char *format, ...);
int vsnprintf(char *buffer, size_t size, const char *format, va_list arguments);

#endif
