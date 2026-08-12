# tremula

[English](README.md) | [한국어](README.ko.md) | **日本語**

**コードに現実的なバグを植えて — テストが震えるか確かめましょう。**

tremulaは外部で定義されたミュータントをテストスイートに対して実行し、
どれが生き残るかを報告します。生き残ったミュータントは、テストがカバー
していない隙間の証拠です。

> [!NOTE]
> 早期開発段階です。パッケージはまだ公開されておらず、インターフェースは
> 変更される可能性があります。

## なぜtremulaか

- **何を変異させるかはあなたが決めます。** manifestがファイル、正確な
  バイト範囲、置換内容を指定します。手書きでもモデルに依頼しても構いませ
  ん。誰も頼んでいないミュータントを吐き出すパターンスキャナーではあり
  ません。
- **「killed」の定義は一つです。** Rustのcoreが中立なシグナルからすべての
  判定を下し、言語パックはスイートを実行して起きたことを報告するだけで
  す。2つの言語が検出の意味を食い違えることはありません。
- **サバイバーはモデルの言葉ではなく実験で分類します。** `tremula triage`
  はサバイバーを区別できる入力をモデルに尋ね、その入力を関数の両バージョ
  ンに対して実行し、それぞれの結果を報告します。
- **引き渡せる証拠を作れます。** `tremula bundle`は実行のreport、patch、
  ログをハッシュ索引付きのディレクトリにパッケージ化し、同僚やコーディン
  グエージェントが再現できます。

## ソースからインストール

tremulaはPython言語パックとCosmic Rayを通じてPython 3.10以降をサポート
しています。どちらのパッケージもまだPyPIにないため、このリポジトリを
クローンして計測したいプロジェクトに両方をインストールしてください:

```sh
uv add --dev --editable /path/to/tremula /path/to/tremula/packs/python
uv run tremula --version
```

これにより、CLIと言語パックがテストスイートと同じプロジェクト環境に
入ります。

## クイックスタート

実行には、何を変異させるかを記述したmanifestが必要です。手書きするか
モデルに依頼し、検証してから実行してください:

```sh
export OPENAI_API_KEY=...
uv run tremula generate --file schedule.py --function overlaps \
  --tests test_schedule.py --model gpt-5.2-2025-12-11
uv run tremula validate --manifest tremula-manifest.json
uv run tremula run --manifest tremula-manifest.json
```

```text
tremula run · 3 mutants · project: .
baseline: 3 passed in 0.8s ✓ (collected=3)

  id        file             span      verdict    detail
  0d7ba921  schedule.py      247–264   KILLED     1 failed
  96c8983f  schedule.py      395–408   SURVIVED   3 passed
  1f3a9d2e  schedule.py      511–528   SURVIVED   3 passed

score: 1/3 killed (0 timeout) · 2 survived · 0 excluded
note: SURVIVED = not killed by the existing suite (execution/coverage unverified)
exit 1 (survived present)
next: 1. `tremula triage --model <model>` sorts the survivors, then 2. `tremula bundle` packages this run's evidence for somebody else — in that order, so the triage travels with it
run_dir=/path/to/project/.tremula/runs/20260809T041500Z-3b1f8c
```

`0`は生き残りがいないこと、`1`はいること、`2`はその実行が信頼できない
ことを意味します。生き残りがある場合は、読む前に分類し、証拠をパッケージ
化してください:

```sh
uv run tremula triage --model gpt-5.2-2025-12-11
uv run tremula bundle
```

> [!WARNING]
> **モデルへの依頼には費用がかかり、指定したファイルがそのプロバイダーに
> 送信されます。** ネットワーク呼び出しを行うのは`generate`と`triage`
> だけです。
>
> **manifestはこれから実行するコードです。** すべての`replacement`は
> テストスイートの一部として実行されるため、信頼できるmanifestだけを
> 実行してください。
>
> **ミュータントはその場で適用されます。** 実行が強制終了されるとソースに
> 残ることがあります。`tremula restore`が実行のスナップショットから元に
> 戻し、ソースが変更されたまま失敗した実行は、その実行用の正確なコマンド
> を表示します。

プロジェクトの`.gitignore`に`.tremula/`と`tremula-bundle-*`を追加して
ください。実行結果はプロジェクト内に書き込まれ、コミットすると以後の
すべての実行が変更されたワークツリーを見ることになります。

## 次に見るもの

| やりたいこと | ガイド |
| --- | --- |
| 復旧まで含めて初回実行を完了する | [はじめてのミューテーションテスト](docs/guides/first-run.md) |
| triage結果を理解しサバイバーをdismissする | [サバイバーの確認とdismiss](docs/guides/survivor-review.md) |
| 同僚やコーディングエージェント向けに実行結果をパッケージ化する | [証拠バンドルの共有](docs/guides/bundle-sharing.md) |
| アーキテクチャ、コントラクト、決定記録 | [ドキュメント索引](docs/README.md) |

生成されたJSON Schemaと共有例は[`contracts/`](contracts/README.md)に
あります。リンク先のドキュメントは英語です。

## 開発

リポジトリをセットアップし、CIで使われるのと同じチェックを実行します:

```sh
uv sync
just lint
just test
just e2e
```

Rustのコントラクト型を変更したら`just contracts`を実行してください。
`just build`は配布可能なwheelを`dist/`に作成します。

## ライセンス

MIT
