#ifndef JAMSCRIPT_SCRIPTC_STDLIB_H
#define JAMSCRIPT_SCRIPTC_STDLIB_H
#include <stddef.h>
void *malloc(size_t);
void *calloc(size_t, size_t);
void *realloc(void *, size_t);
void free(void *);
_Noreturn void abort(void);
char *getenv(const char *);
long strtol(const char *, char **, int);
double strtod(const char *, char **);
#endif
