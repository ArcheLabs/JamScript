#ifndef JAMSCRIPT_M0_STDLIB_H
#define JAMSCRIPT_M0_STDLIB_H

#include <stddef.h>

void *malloc(size_t size);
void *calloc(size_t count, size_t size);
void *realloc(void *pointer, size_t size);
void free(void *pointer);
_Noreturn void abort(void);

#endif
