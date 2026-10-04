# Implementation plan

This document owns current status, acceptance evidence, release gates, and open
decisions. The [design](design.md) owns architecture. The [usage guide](usage.md)
owns command details. The [check catalog](checks.md) owns the wider accepted
scope. Catalog entries are not proof of implementation.

## Current state

Perfect Doc has a working validator and command. The implementation covers a
useful subset of the accepted structural checks. It does not implement full HTML
conformance, computed accessibility, browser rendering, or all catalog rules.

| Area | State |
| --- | --- |
| Rust library and command | Implemented; the library returns reports and the command handles output and exit codes. |
| Offline scan | Implemented for supported Markdown, HTML, metadata, references, assets, and collection settings. |
| Static includes | Implemented as an opt-in feature with original source locations and combined anchors. |
| Online HTTP checks | Implemented as an opt-in feature with limits and an optional explicit cache. |
| Reports | Human, JSON, JUnit, and SARIF formats are available. |
| Python and npm packages | Local build paths are implemented. They are not published. |
| Full accepted check catalog | Incomplete; see [usage limits](usage.md) and [checks](checks.md). |
| Other operating systems and CPUs | Not verified at runtime. |
| Remote repository | Public source is hosted at `btfranklin/perfect-doc` on GitHub. |
| Hosted CI and package publication | Not configured. |

Each diagnostic has a source location, rule, result, message, and repair help.
It has a target when the check has one. Reports redact user information and
query values in URI targets. The output contract is described in the
[design](design.md).

## Local verification

The local host is Apple Silicon macOS. The native executable, Python wheel, and
npm packages have been built and run on this host. Package metadata tests cover
the configured target shapes, but they do not prove that other targets run.
Do not claim Windows or Linux runtime support from these checks.

The local source checks passed:

- 210 Rust tests, including property tests, controlled HTTP tests, source map
  checks, report checks, and public library and command tests.
- 12 Python package and launcher tests.
- 140 repeated HTTP fixture runs with concurrent test processes after the
  accepted socket handling fix.
- Python, Node.js, and Make integration examples with the native executable.
- Rust formatting, Clippy with warnings denied, and API documentation.
- The repository's own documentation scan.
- GitHub source installation in a temporary prefix, followed by an offline
  scan with the installed executable.

The locally verified tool versions were:

| Tool | Version |
| --- | --- |
| Rust and Cargo | `1.99.0` |
| Python | `3.14.8` |
| PDM | `2.29.2` |
| pytest | `9.1.1` |
| Maturin | `1.15.0` |
| Node.js | `26.10.0` |

These are local observations from 2026-10-03 in `America/Phoenix`. Check tool
versions again before a release.

Use the dependency setup and validation commands in the
[README](../README.md#local-checks). Run them from the repository root. The
README owns these commands; this plan records their acceptance evidence.

The language examples call the same native executable. They do not contain a
second validator. The Python and npm build instructions are in the
[usage guide](usage.md). Build artifacts stay in ignored `dist/` and `target/`
directories.

## Capability work

The original staged plan is now a status map. Core scanning, configuration,
reports, collection checks, includes, and bounded HTTP checks exist. Work remains
to expand format coverage and test each accepted rule against valid, invalid,
and ambiguous structures. Do not use the catalog family count as a measure of
implementation coverage.

Known limits include:

- HTML checks cover selected syntax and static relationships. They do not cover
  the full HTML or ARIA standards, computed accessible names, or rendering.
- MDX and template expressions are not executed. Dynamic structure can remain
  unverified.
- Online checks use HTTP and HTTPS. They do not execute remote code or prove
  future availability.
- Includes require configuration. Their source maps and combined anchors have
  local tests, but further format and nested edge cases remain part of ongoing
  validation.
- Source maps support UTF-8. A selected other HTML encoding is reported as
  unsupported. Includes require static local file targets and UTF-8 slices.
- Image decoding supports the formats enabled in `Cargo.toml`. Other detected
  image formats are reported as unsupported. Audio, video, and font checks cover
  selected headers and reference types, not full media decoding.
- CSS checks parse tokens and references. They do not validate every property or
  stylesheet rule against the complete CSS specifications.
- XML syntax checks do not implement XML schema validation. A JSON Schema
  contract for XML is reported as unsupported.
- The accepted catalog contains checks that the current implementation does
  not provide.

Update this plan when support or evidence changes. Record the command, host
scope, and result. Do not turn a package shape check into runtime evidence.

## Distribution and release gates

The Python wheel uses Maturin `bin` mode. The npm builder creates a private
launcher package and one private native package per selected target. Both call
the same executable. The configured targets are:

| Rust target | npm platform package suffix | Runtime evidence |
| --- | --- | --- |
| `aarch64-apple-darwin` | `darwin-arm64` | Built and run locally. |
| `x86_64-apple-darwin` | `darwin-x64` | Not run. |
| `x86_64-unknown-linux-gnu` | `linux-x64-gnu` | Not run. |
| `x86_64-unknown-linux-musl` | `linux-x64-musl` | Not run. |
| `x86_64-pc-windows-msvc` | `win32-x64` | Not run. |

The package names are candidates. Registry availability is unknown. The Cargo
manifest sets `publish = false`. The owner selected the [MIT license](../LICENSE).
Cargo and Python metadata declare MIT. Both npm packages declare MIT and include
the license text. No wheel or npm package has been published.

Before publication, verify package names, select release identities, and obtain
the owner's authorization for registry access. Build and run each
claimed platform package on its target system. Verify offline use, dependency
requirements, package contents, and recovery instructions. Do not configure a
remote, hosted CI, or publication workflow without a user request.
