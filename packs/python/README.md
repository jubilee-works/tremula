# tremula-python

The Python language pack for [tremula](https://github.com/jubilee-works/tremula),
an AI-assisted mutation testing tool. This package runs a tremula manifest
against a pytest suite through Cosmic Ray and reports which mutants survived.

It is installed automatically as a dependency of the `tremula` package, so
install that instead of this package directly:

```sh
pip install tremula
```

See the [tremula README](https://github.com/jubilee-works/tremula#readme) for
usage, and the
[pack protocol](https://github.com/jubilee-works/tremula/blob/main/contracts/pack-protocol.md)
for how the core drives a language pack.

## License

MIT. Developed by [TimeTree](https://timetreeapp.com).
