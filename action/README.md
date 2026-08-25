# Tremula composite Action

This composite Action runs tremula's pull-request path: it selects changed functions, generates mutations, runs them, triages survivors, uploads their evidence, and updates a pull-request comment.

## Usage

Check out the repository that contains this scaffold, then invoke it from a pull-request workflow:

```yaml
name: Tremula

on:
  pull_request:

jobs:
  tremula:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      pull-requests: write
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: ./action
        with:
          version: "0.1.0"
          model: gpt-5.2-2025-12-11
          coverage: coverage/lcov.info
        env:
          OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}
```

`version` and `model` are required. `coverage` and `max-functions` are optional. By default, `diff-base` is `origin/${{ github.base_ref }}`, and `github-token` is the workflow token.

## Availability

This Action only works after `tremula` has been published to PyPI: it installs the requested version with `pip install "tremula==<version>"`.

It is intentionally scaffolded inside this repository for now. Extraction to a separate Action repository is planned for later.
