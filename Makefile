.PHONY: lint shellcheck shellcheck-skills markdownlint jsonlint actionlint clippy fmt setup cargo-test cargo-clippy

lint:
	pre-commit run --all-files

shellcheck:
	pre-commit run shellcheck --all-files

shellcheck-skills:
	scripts/shellcheck-scripts.sh

markdownlint:
	pre-commit run markdownlint --all-files

jsonlint:
	pre-commit run jsonlint --all-files

actionlint:
	pre-commit run actionlint --all-files

cargo-test:
	cargo test

cargo-clippy:
	cargo clippy -- -D warnings

clippy:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt -- --check

setup:
	pre-commit install
