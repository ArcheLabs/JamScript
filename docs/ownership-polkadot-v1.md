# Polkadot Ownership Binding v1

SS58 is a client-only human representation. The client decodes SS58 to AccountId32 and represents it as `Ownership(MULTICRYPTO_ACCOUNT32, AccountId32)`; SS58 text never enters consensus.

The JamScript adapter preserves Polkadot-compatible Ed25519, Sr25519, and ECDSA verification. The signature scheme is part of the authorization proof, while the resulting AccountId32 remains the Ownership public representation.
