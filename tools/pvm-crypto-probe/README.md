# Crypto PolkaVM probe

The crypto probe covers the no-std signing/verification primitives used by a
generated service. The fixed vector covers public-key decode, signature
decode, Merlin transcript construction, and sr25519 verification. Its
acceptance artifact is the canonical `service.elf` plus interpreter execution;
no host-only crypto implementation is accepted as a substitute.
