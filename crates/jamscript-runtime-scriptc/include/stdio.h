#ifndef JAMSCRIPT_SCRIPTC_STDIO_H
#define JAMSCRIPT_SCRIPTC_STDIO_H
#include <stddef.h>
#include <stdarg.h>
typedef struct JamscriptScriptcFile FILE;
extern FILE *stdout;
extern FILE *stderr;
int snprintf(char *, size_t, const char *, ...);
int vsnprintf(char *, size_t, const char *, va_list);
int fflush(FILE *);
int fputs(const char *, FILE *);
int fputc(int, FILE *);
size_t fwrite(const void *, size_t, size_t, FILE *);
#endif
