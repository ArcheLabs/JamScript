# State Recovery Formats

Design only — recovery infrastructure is not implemented by the Runtime.

`RecoveryRecordV1` fixes the future reconstruction envelope:

```text
version
service_id
parent_root
new_root
code_hash
state_delta
```

`StateDiffV1` has stable encoding and a deterministic hash. A future recovery
worker can apply deltas to a checkpoint and compare every calculated root.
Full execution replay may instead use historical Service code, transactions
and external witnesses. The wire format leaves both approaches possible.

Recovery records are reconstruction aids, not additional consensus-critical
commitments. Only the Managed State root committed through JAM is canonical.

Persistent availability is not guaranteed merely because a Work executed.
Future infrastructure may use D3L exported segments, rolling checkpoints,
archive providers or additional persistent DA. Audit DA and long-lived
exported-segment retention have different protocol lifetimes; the Runtime does
not claim indefinite recovery from JAM alone.
