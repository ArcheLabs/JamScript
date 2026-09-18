# JamScript release process

Release identity is the pair of an exact source commit and an immutable tag.
The process is deliberately split into qualification and publication:

```text
candidate commit → normal CI → manual Release from main
                 → validate/toolchain/CLI production jobs
                 → automatic immutable tag → exact-byte publication
                 → JAMSCRIPT_RELEASE_PUBLISHED=PASS
```

## Prepare

Update `main`, confirm a clean worktree, and record the candidate commit. All
correctness, consumer, determinism, and host-environment checks must already be
green in ordinary CI before starting a release.

## Release validation

Run `Release JamScript` manually from `main` with the intended version, for
example `v0.1.0-rc.4`. The workflow has only four jobs: validate, build one
toolchain archive per platform, build one CLI archive per platform, and publish
the exact bytes. Validation requires an already successful push-triggered CI
run for the exact source SHA; it does not rerun those tests. It performs only
cheap archive-structure checks before publication. No consumer build,
execution closure, determinism comparison, Backend binary, Docker image, GHCR
push, or backend health check is part of this workflow.

## Freeze and publish

After the producer jobs pass, `release.yml` creates one annotated immutable tag
on the validated commit and publishes the exact producer bytes. The public
release contains only two CLI archives, two managed toolchain archives, and
`SHA256SUMS`. Existing GitHub Releases are rejected rather than replaced.

## Accept

The JamScript release is complete when the publish job confirms the exact tag
and GitHub Release creation and prints `JAMSCRIPT_RELEASE_PUBLISHED=PASS`.
Consumer behavior is a CI responsibility. Backend publication is a separate
`Release Backend` workflow with its own tag and readiness marker.

## Failure classification

Class A is a transient infrastructure failure: a GitHub service error, a
network timeout, an unavailable runner, a temporary artifact backend failure,
or a transient Release API error. If the immutable tag and source are still
correct, rerunning that same tag is allowed.

Class B is a source or release-engineering failure: compiler or workflow bugs,
LLVM lock mismatches or zero sentinels, missing assets, manifest or hash
mismatches, and reproducibility failures. Keep the failed tag for history, do
not mutate or delete it, fix the source or workflow in a new commit, and
qualify the next release candidate. Consumer validation failures are classified
as CI failures; backend runtime failures are classified under the independent
Backend Release workflow.

The same rule applies to stable `v0.1.0`: it may be tagged only after all
JamScript CI gates pass. Merging `main` never creates a release tag
automatically. Backend tags use the separate `backend-v<semver>` namespace and
are governed by `backend-release.yml`.
