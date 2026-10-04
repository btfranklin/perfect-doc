# Perfect Doc

Perfect Doc checks the structure of Markdown, OKF, and HTML documents. It checks
supported links, anchors, metadata, assets, and configured document rules.
It does not grade prose or execute document code.

Extract this archive. Put the `perfect-doc` executable on your `PATH`. On
Windows, the executable is `perfect-doc.exe`. The executable does not require
Rust, Python, or Node.js.

Run these commands from the project that owns your documents:

```sh
perfect-doc --version
perfect-doc check . --offline --no-banner
```

The scan writes its report to standard output. Exit code `0` means that the
selected required checks passed at the configured failure level. Code `1`
means that a document is invalid. Code `2` means that the command could not
run. Code `3` means that a required check is unsupported or unverified.

This archive includes the project license in `LICENSE` and dependency license
notices in `THIRD-PARTY-NOTICES.txt`.

See the [usage guide](https://github.com/btfranklin/perfect-doc/blob/main/docs/usage.md)
for configuration, report formats, and current limits. Get native builds and
their checksums from [GitHub Releases](https://github.com/btfranklin/perfect-doc/releases).
