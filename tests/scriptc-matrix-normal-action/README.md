# Matrix normal-action PVM regression

This gate covers the ordinary Matrix device path separately from Matrix
bootstrap authorization:

- Test A signs and executes a direct Ed25519 `SignedActionV2` in PVM.
- Test B signs as device D while acting for subject M, with active, missing,
  and revoked controller grants.
- Set `LOCUS_ROOT` to a Locus source checkout to qualify direct transfer,
  delegated Matrix transfer, pool creation, and swap against real Locus actions
  and canonical state.

Run the complete gate with the contributor toolchain and a Locus checkout:

```sh
JAMSCRIPT_DEV_TOOLCHAIN=1 LOCUS_ROOT=/path/to/locus \
  scripts/test-scriptc-matrix-normal-action-pvm.sh
```

The real-Locus gate checks action receipts and state roots, then verifies the
sender/recipient balances, pool reserves, and minted LP position. It builds a
fresh local PVM artifact; it does not publish or deploy a Service.

## Root-cause evidence

Before the fix, Test A and Test B passed. In the full Locus PVM, direct
ownership transfer passed, while delegated Matrix transfer trapped in
`jamscript_plan_v1` at PC `226918` (`0x37666`). The artifact had BLAKE2 digest
`03011f66195dcbf832707d67905e59c804ccb2230090e1463d864595b7480842` and
canonical code hash `bbd1b82bf9e26f98cf191b7b3c80d4584c464675c52b669c654ba1454ca920b7`.

A diagnostic guest build measured a `296625` byte peak for delegated Matrix
transfer, above the former `262144` byte per-invocation arena. Direct transfer
peaked at `173332` bytes. The production arena is now 512 KiB; this PVM gate
will trap again if a future change makes the valid flow exceed that bound.
