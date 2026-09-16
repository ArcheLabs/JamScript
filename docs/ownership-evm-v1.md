# EVM Ownership Binding v1

An EVM address is normalized client-side to `Ownership(SECP256K1_KECCAK20, H160)`. EVM ownership is direct: `controller=owner` and `act_as` is absent unless a separate explicit claim is used.

JamScript authentication uses EIP-712 with domain name `JamScript`, version `1`, and the network domain as salt. The adapter accepts valid low-s recovery signatures and checks that the recovered public key hashes to the Ownership H160. EIP-712 is an authentication adapter, not part of Ownership Core.
