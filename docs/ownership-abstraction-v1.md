# Ownership Abstraction v1

Ownership is the canonical cryptographic public control identifier. It is not an identity, username, account balance, transaction, or authorization proof.

`OwnershipV1` is encoded as `version:u8 || kind:u8 || public_length:u16(le) || public_bytes`. Version 1 defines `ED25519_KEY=0`, `SR25519_KEY=1`, `SECP256K1_KEY=2`, `SECP256K1_KECCAK20=3`, and `MULTICRYPTO_ACCOUNT32=4`. Public lengths are respectively 32, 32, 33, 20, and 32 bytes and are validated exactly.

`OwnershipKey` is derived only for indexes and nonce lanes:

```text
BLAKE2b-256("OWNERSHIP_ABSTRACTION_KEY_V1" || canonical_ownership)
```

Ownership is not OwnershipKey. Matrix, EVM, and Polkadot are adapters that resolve external representations into this primitive; they do not define new Ownership kinds.
