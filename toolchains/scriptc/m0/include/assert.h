#ifndef JAMSCRIPT_M0_ASSERT_H
#define JAMSCRIPT_M0_ASSERT_H

_Noreturn void __assert_fail(const char *, const char *, unsigned, const char *);
#define assert(expression) ((expression) ? (void)0 : __assert_fail(#expression, __FILE__, __LINE__, __func__))

#endif
