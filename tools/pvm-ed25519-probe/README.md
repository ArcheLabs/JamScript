# Ed25519 PolkaVM probe

This probe slot is reserved for a minimal guest that exercises the runtime's
Ed25519 verification path under the official PolkaVM target. It must build
through `service-build-polkavm`, use the pinned 0.30 converter, and remain
independent of the browser/client path.
