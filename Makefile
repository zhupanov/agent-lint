.PHONY: lint shellcheck shellcheck-skills markdownlint jsonlint actionlint rust-check fmt setup test-check-bump-version test-release-agent-lint test-rust-check test-upgrade-agent-lint

lint: test-check-bump-version test-release-agent-lint test-rust-check test-upgrade-agent-lint
	pre-commit run --all-files

test-check-bump-version:
	bash scripts/test-check-bump-version.sh

test-release-agent-lint:
	bash scripts/test-release-agent-lint.sh

test-upgrade-agent-lint:
	bash scripts/test-upgrade-agent-lint.sh

test-rust-check:
	bash scripts/test-rust-check.sh

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

rust-check:
	bash scripts/rust-check.sh

fmt:
	cargo fmt -- --check

setup:
	pre-commit install
