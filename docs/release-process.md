# JamScript release process

Release identity is the pair of an exact source commit and an immutable tag.
The process is deliberately split into qualification and publication:

```text
candidate commit → normal CI → Release Preflight
                 → RELEASE_PREFLIGHT_READY=PASS
                 → immutable tag → Release Candidate workflow
                 → published-byte validation → RELEASE_READY=PASS
```

## Prepare

Update `main`, confirm a clean worktree, and record the candidate commit. Do
not create a release tag while source, workflow, lock, manifest, or
reproducibility checks are still being repaired.

## Preflight

Run `Release Preflight` manually with an exact candidate SHA (or a branch ref
that is known not to move) and the intended version, for example
`v0.1.0-rc.3`. Preflight has `contents: read` permission only. It rebuilds both
native toolchains twice, validates the exact LLVM locks, builds both CLI
archives, creates candidate `release-manifest.json` and `SHA256SUMS`, runs
fresh Linux and macOS consumer tests, and compares host-independent outputs.

If `RELEASE_PREFLIGHT_READY=PASS` is absent, do not create or push a tag. A
new source or release-engineering fix requires a new candidate commit and a
new preflight.

## Freeze and publish

After preflight passes, create one annotated immutable tag on the validated
commit and push only that tag. `release-candidate.yml` checks out the exact tag
and reproduces all assets; it does not consume a branch build artifact. GitHub
Release creation is the final publication side effect, after all prepublish
gates and cross-host comparisons pass. Existing release assets are downloaded
and compared byte-for-byte; they are never replaced.

## Accept

The release is complete only when native Linux and macOS jobs validate the
public GitHub Release bytes and the final job prints
`RELEASE_READY=PASS`. A GitHub Release existing by itself is not acceptance.

## Failure classification

Class A is a transient infrastructure failure: a GitHub service error, a
network timeout, an unavailable runner, a temporary artifact backend failure,
or a transient Release API error. If the immutable tag and source are still
correct, rerunning that same tag is allowed.

Class B is a source or release-engineering failure: compiler or workflow bugs,
LLVM lock mismatches or zero sentinels, missing assets, manifest or hash
mismatches, reproducibility failures, and consumer-validation failures. Keep
the failed tag for history, do not mutate or delete it, fix the source or
workflow in a new commit, and qualify the next release candidate.

The same rule applies to stable `v0.1.0`: it may be tagged only after a full
passing preflight. Merging `main` never creates a release tag automatically.
