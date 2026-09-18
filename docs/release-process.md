# JamScript release process

Release identity is the pair of an exact source commit and an immutable tag.
The process is deliberately split into qualification and publication:

```text
candidate commit → normal CI → manual Release from main
                 → validate/build/prepublish gates
                 → automatic immutable tag → exact-byte publication
                 → published-byte validation → RELEASE_READY=PASS
```

## Prepare

Update `main`, confirm a clean worktree, and record the candidate commit. Do
not create a release tag while source, workflow, lock, manifest, or
reproducibility checks are still being repaired.

## Release validation

Run `Release` manually from `main` with the intended version, for example
`v0.1.0-rc.3`. The workflow binds all jobs to `github.sha`, rebuilds both
native toolchains twice, validates the exact LLVM locks, builds the CLI and
backend once, creates `release-manifest.json` and `SHA256SUMS`, runs fresh
Linux and macOS consumer tests, compares host-independent outputs, and runs a
prepublish backend image smoke. No tag or release side effect occurs before
these gates pass.

## Freeze and publish

After all prepublish gates pass, `release.yml` creates one annotated immutable
tag on the validated commit and pushes it. The same job publishes the already
validated release assets and the backend image; it does not check out the new
tag or rebuild the release. Existing release assets are downloaded and
compared byte-for-byte; they are never replaced.

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

The same rule applies to stable `v0.1.0`: it may be tagged only after all
release gates pass. Merging `main` never creates a release tag automatically.
