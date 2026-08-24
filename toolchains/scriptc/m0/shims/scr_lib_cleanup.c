/* M0-only profile shim. The scalar fixture has no reachable process/fs API,
 * so its library-session interned-value set is provably empty. Any future M0
 * fixture that reaches process values must reject this shim and provide the
 * real ScriptC cleanup implementation. */
void scr_lib_session_cleanup(void) {}
