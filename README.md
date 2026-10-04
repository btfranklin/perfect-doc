# Perfect Doc

Perfect Doc is a Rust command-line tool and library that checks the structure
of documentation collections. Use it in a unit test, a local test command, or
any workflow that can run an executable and check its exit code.

It does not judge spelling, prose quality, facts, topic coverage, or code
behavior. It does not execute document code or use an AI service.

## Supported checks

The default scan finds Markdown, HTML, OKF, and MDX files. OKF uses the Markdown
reader. Supported checks include:

- Local links, anchors, reference definitions, and selected file-name rules.
- Declared heading structure, tables, code block metadata, and static includes.
- YAML, TOML, and JSON front matter, structured examples, and local JSON Schema
  contracts for supported data formats.
- Selected HTML structure, attributes, ID references, forms, and static
  accessibility relationships.
- Local assets, routes, aliases, navigation, and supplied generated output.

Checks that need a project contract, such as required metadata or heading
levels, run when that contract enables them. HTTP and HTTPS link checks are
optional; the default scan is offline.

The current implementation does not provide full HTML or ARIA conformance,
computed accessibility, or browser and renderer execution. Dynamic structure
can remain unverified. See [supported checks and limits](docs/usage.md#supported-checks-and-limits)
and run `perfect-doc rules` to list the supported rule IDs.

## Run a check

With Rust 1.99 or later installed, run a check from this directory:

```sh
cargo run --locked --release -- check .
```

The command reads `perfect-doc.toml` when it exists in the current directory.
Otherwise it uses the default configuration. This repository has a
[structural contract](perfect-doc.toml) that checks its own documentation. Use
`perfect-doc init` to write a default configuration file. The command does not
replace an existing file.

Reports go to standard output by default. Each finding includes its source
location, rule, result, message, repair help, and target when one applies.
Reports can use human-readable, JSON, JUnit, or SARIF output. A scan does not
write a report file or cache unless you set `--output` or configure an online
cache path. See [output and exit codes](docs/usage.md#output-and-exit-codes).

Human-readable command output starts with a built-in ASCII outline of
Perfect Doc, based on the Perfect Dark (BRK) font. Use `--no-banner` to hide it:

```sh
perfect-doc check . --no-banner
```

Machine reports, configuration output, and `--version` do not include the
banner. The font is not required at run time.

## Build and install

Build and run the native executable with Cargo:

```sh
cargo build --locked --release --bin perfect-doc
./target/release/perfect-doc check .
```

Use the release build for repeat scans. You can install it from this checkout
with `cargo install --locked --path .`, then run `perfect-doc` from the project
that owns the documents. The native executable needs no Python or Node.js
runtime.

Local build workflows also produce a Python wheel and npm packages. They call
the same native executable. The packages are not published. Native, Python,
and npm installs have been run on Apple Silicon macOS; other targets have not
been verified at runtime. See [build and install](docs/usage.md#build-and-install)
and the [release status](docs/plan.md#distribution-and-release-gates), including
the remaining package release checks.

## Test integration

Any test system can call `perfect-doc check .` and use the exit code. An invalid
result can fail a test at the configured severity. A required unsupported or
unverified check also returns a nonzero code.

The [Python, Node.js, and Make examples](examples/integration/README.md) show
this process boundary. Rust tests can call the library directly; see
[test integration](docs/usage.md#test-integration).

## Local checks

To run all local checks, install Rust, PDM, Python 3.11 or later, Node.js, and
Make. Prepare the dependencies once from the repository root:

```sh
cargo fetch --locked
pdm install --dev --no-self
```

Then run the source checks and integration examples:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked --offline -- -D warnings
cargo test --locked --offline
cargo doc --no-deps --locked --offline
pdm run python -m unittest discover -s tests -p 'test_packaging.py'
PERFECT_DOC="$PWD/target/debug/perfect-doc" pdm run pytest examples/integration/test_docs.py
PERFECT_DOC="$PWD/target/debug/perfect-doc" node --test examples/integration/docs.test.mjs
make -C examples/integration check PERFECT_DOC="$PWD/target/debug/perfect-doc"
make -C examples/integration check-cargo
```

The HTTP tests start temporary servers on the loopback interface. Perfect Doc
itself is a command and library; it does not start a server. Current test
evidence is in the [implementation plan](docs/plan.md#local-verification).

## Project documents

- [Usage](docs/usage.md) explains commands, configuration, reports, and limits.
- [Design](docs/design.md) explains architecture and product boundaries.
- [Check catalog](docs/checks.md) records all accepted check families. Catalog
  entries do not prove that every check is implemented.
- [Implementation plan](docs/plan.md) records current capability and evidence.
- [Agent guide](AGENTS.md) records project work rules.

## License

Perfect Doc uses the [MIT license](LICENSE).
