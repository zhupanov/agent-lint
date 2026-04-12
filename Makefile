.PHONY: lint shellcheck markdownlint jsonlint actionlint clippy fmt setup

lint:
	pre-commit run --all-files

shellcheck:
	pre-commit run shellcheck --all-files

markdownlint:
	pre-commit run markdownlint --all-files

jsonlint:
	pre-commit run jsonlint --all-files

actionlint:
	pre-commit run actionlint --all-files

clippy:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt -- --check

setup:
	pre-commit install
