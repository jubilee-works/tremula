default: lint test

lint:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    uv run --package tremula-python ruff check packs/python
    uv run --package tremula-python pyright packs/python

test:
    cargo test
    uv run --package tremula-python pytest packs/python/tests

build:
    uvx maturin build --out dist

e2e:
    uv sync
    uv run tremula --version
