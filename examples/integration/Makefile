PERFECT_DOC ?= perfect-doc

.PHONY: check check-cargo

check:
	"$(PERFECT_DOC)" check docs

check-cargo:
	cargo run --locked -- check docs
