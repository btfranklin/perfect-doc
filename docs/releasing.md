# Release Perfect Doc

This guide owns the release procedure. `Cargo.toml` owns the package version.
The [implementation plan](plan.md) owns acceptance evidence and publication
state. The [release notes](release-notes.md) describe the current release.

## Prepare the release

1. Update the package version in `Cargo.toml` when a new release needs one. Let
   Cargo update `Cargo.lock` for that change. Python and npm builders read the
   Cargo version.
2. Update the current release notes. Describe supported checks, target systems,
   and limits. Do not claim runtime support from a file signature or metadata
   test.
3. Run the [local checks](../README.md#local-checks). Commit and push the release
   source to `main`.
4. Wait for all four native jobs in the
   [CI workflow](../.github/workflows/ci.yml). Correct failures before tagging.

CI uses stable Rust, the latest stable Python and Node.js, and the locked project
dependencies. Action revisions are pinned to verified official releases. The
Linux runner uses Ubuntu 24.04 as its build baseline. Both macOS runners use
macOS 15 and deployment target 15.0. The Windows runner uses Windows Server 2025
and links the C runtime statically.

## Tag and build

Check that the tag is unused, then tag the tested commit. For the first release:

```sh
git tag -a v0.1.0 -m "Perfect Doc 0.1.0"
git push origin v0.1.0
```

The [release workflow](../.github/workflows/release.yml) calls the same CI
workflow from the tagged commit. It checks the tag against the Cargo version.
Each native job builds an archive, extracts it, and runs both a valid scan and
a broken-link scan with the extracted executable. The
[archive builder](../scripts/release-artifacts.py) owns archive layout, binary
signature checks, version checks, and SHA-256 files.

After all target checks pass, the workflow verifies the archive checksums and
creates a draft GitHub release with four archives and `SHA256SUMS`. Only this
draft job has repository write permission. The workflow does not publish a
release or write to a package registry.

## Publish and verify

Review the draft notes, target checks, archive names, and checksums. Confirm that
the tag points to the tested source. Publish the reviewed draft:

```sh
gh release edit v0.1.0 --repo btfranklin/perfect-doc --draft=false
```

Download the published archives from GitHub and verify their SHA-256 checksums.
Run the archive for an available native target and check its version and scan
behavior. Record the hosted checks and public download evidence in the plan.

Homebrew formula updates, bottles, and registry publication have separate
release steps. Do not start them as part of this native release procedure.
