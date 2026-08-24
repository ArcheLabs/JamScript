#ifndef JAMSCRIPT_M0_STDIO_H
#define JAMSCRIPT_M0_STDIO_H

#include <stddef.h>
#include <stdarg.h>

typedef struct JamscriptM0File FILE;
extern FILE *stdout;
extern FILE *stderr;

int snprintf(char *buffer, size_t size, const char *format, ...);
int vsnprintf(char *buffer, size_t size, const char *format, va_list arguments);
int fflush(FILE *stream);
int fputs(const char *text, FILE *stream);
int fputc(int character, FILE *stream);
size_t fwrite(const void *data, size_t size, size_t count, FILE *stream);

#endif
