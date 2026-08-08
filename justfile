default: lint test

lint:
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings
    uv run --package tremula-python ruff check packs/python
    uv run --package tremula-python pyright --project packs/python packs/python

test:
    cargo test
    uv run --package tremula-python pytest packs/python/tests

contracts:
    TREMULA_UPDATE_CONTRACTS=1 cargo test -p tremula-contracts --test schemas

build:
    uvx maturin build --out dist

e2e:
    uv sync
    INSTA_UPDATE=no cargo test --test e2e --features e2e
