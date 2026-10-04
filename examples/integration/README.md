# Test integrations

These examples call the installed Perfect Doc command. They use the local
`docs` folder as a small test fixture. Run them from this directory. Install
pytest to run the Python example. The Node.js example uses the built-in test
runner.

Run the Python example with `python -m pytest`. Run the Node.js example with
`node --test docs.test.mjs`. Run `make check` to use the installed command, or
run `make check-cargo` to build and call the command with Cargo.

Set `PERFECT_DOC` to the executable's absolute path when it is not on `PATH`.
The repository root [README](../../README.md#local-checks) has commands for
all three examples.
