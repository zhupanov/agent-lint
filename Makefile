.PHONY: lint shellcheck shellcheck-skills markdownlint jsonlint actionlint clippy fmt setup cargo-test cargo-clippy test-check-bump-version test-upgrade-agent-lint

lint: test-check-bump-version test-upgrade-agent-lint
	pre-commit run --all-files

test-check-bump-version:
	bash scripts/test-check-bump-version.sh

test-upgrade-agent-lint:
	bash scripts/test-upgrade-agent-lint.sh

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
