# Perfect Doc 0.1.0

This is the first native release of Perfect Doc. It checks the structure of
documentation collections from a command or a Rust library. It does not grade
prose, check spelling, execute document code, or use an AI service.

## Included checks

- Markdown, HTML, OKF, and static MDX structure.
- Local links, anchors, reference definitions, and selected file-name rules.
- Configured heading structure, tables, code block metadata, and static includes.
- YAML, TOML, and JSON front matter, structured examples, and local JSON Schema
  contracts for supported formats.
- Selected HTML structure, ID references, forms, and static accessibility
  relationships.
- Local assets, routes, aliases, navigation, and supplied generated output.
- Optional bounded HTTP and HTTPS checks.

Reports use human-readable, JSON, JUnit, or SARIF output. Findings include a
source location, rule, result, message, and repair help. Normal scans write to
standard output. Offline scans are the default.

## Native downloads

Each archive contains the executable, README, MIT license, and dependency license
notices. Verify its SHA-256 checksum against `SHA256SUMS` before use. Extract the
archive and place the executable in a directory on your `PATH`. Python, Node.js,
and Rust are not needed to run these native executables.

| Archive target | Build and runtime check |
| --- | --- |
| `aarch64-apple-darwin` | macOS 15 on Apple Silicon; deployment target 15.0. |
| `x86_64-unknown-linux-gnu` | Ubuntu 24.04 on x64, with glibc 2.39. |
| `x86_64-pc-windows-msvc` | Windows Server 2025 on x64; the C runtime is linked statically. |

Current macOS support requires Apple Silicon. The original v0.1.0 release also
included an Intel macOS archive. That archive remains available but is no longer
a supported target.

Other operating system versions are not part of the first hosted runtime
checks. Older glibc versions are not verified. A musl archive is not included.

## Known limits

The implementation covers a subset of the accepted check catalog. It does not
provide full HTML or ARIA conformance, computed accessibility, or browser
rendering. Dynamic MDX and template expressions are not executed. A required
unsupported or unverified check returns a nonzero exit code.

See the [usage guide](https://github.com/btfranklin/perfect-doc/blob/v0.1.0/docs/usage.md#supported-checks-and-limits)
for format limits. For language workflows, see the
[integration examples](https://github.com/btfranklin/perfect-doc/blob/v0.1.0/examples/integration/README.md).

## Distribution scope

The Python wheel and npm packages have local build paths. They are not published
to package registries. The
[Homebrew tap](https://github.com/btfranklin/homebrew-tap) provides stable bottles
for Apple Silicon macOS and Linux x64. See its README for installation commands.
