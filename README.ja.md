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

## プルリクエストが変えた箇所だけを変異させる

関数を名指しする代わりに、比較対象のリビジョンを渡せば`generate`が対象を
自分で決めます。テスト実行のカバレッジも渡すと、変更行のうちテストが実際に
通る行が、**どの関数を**尋ねる価値があるかを決めます。変異そのものは選ばれた
関数の中のどこにでも提案されえます — 値打ちのある提案はたいていそちらにあります
— そして、テストが通らない行を置き換える変異は記録される直前に拒否されます:

```sh
uv run coverage lcov -o lcov.info
uv run tremula generate --diff-base origin/main --coverage lcov.info \
  --model gpt-5.2-2025-12-11
uv run tremula run --manifest tremula-manifest.json
uv run tremula comment
```

```text
tremula generate · ranges.py · 2 function(s) · model: gpt-5.2-2025-12-11

selected 2 of 2 functions · 0 capped · 1 test file(s) excluded

  overlaps: 4 proposed · 4 recorded
  merge: 4 proposed · 3 recorded · refused: original_not_found ×1

wrote 7 mutant(s) to ./tremula-manifest.json
tokens: 1364 prompt · 872 completion · 2236 total
exit 0 (7 mutant(s) to run)
```

比較はブランチがベースから分岐した地点から行われるため、ベースがその後
進んでいてもベース自身のコミットがこの変更に混ざりません。
`--max-functions`の既定値は5で、上限で外れた関数は黙って捨てられず
記録されます。`--coverage`なしで実行すると変更された関数がすべて対象に
なり、出力がそのことを明示します — 生き残ったミュータントが「テストが
通っていないから」生き残った可能性があるからです。

**変異させるものが何もない変更は失敗ではなく成功です。** 終了コードは`0`で、
manifestも書き出します。変更行のうちどのテストも通らない箇所がどこかという
記録が、そうした実行のもっとも価値ある成果だからです。`run`はそうした
manifestについてレポートだけを書いて終わるので、CIジョブは分岐なしに
コマンドを並べれば済みます。

各関数が選ばれた理由はmanifestの`selection`に記録され、そこから実行
ディレクトリと証拠バンドルまでそのまま運ばれます。`tremula comment`は
manifestと実行ディレクトリを読んでプルリクエストのコメント本文を出力し、
`--github-pr <N>`を渡すと投稿します — スレッドに積み増す代わりに、自分が
以前残したコメントを置き換えます。詳しくは
[Comments](docs/01-architecture/comments.md)を参照してください。

ミュータントを1つも作れずに緑で終わった実行こそ、注記を付ける価値があります —
終了コードだけでは「何も問題が見つからなかった実行」と区別できないからです。
コアはここでプラットフォーム中立を保ち、GitHubのワークフローであれば、
`generate`が必ず書き出すmanifestを読んで1ステップで可視化できます:

```yaml
- name: Warn when a generation produced no evidence
  run: |
    barren=$(jq '[.selection.functions[] | select(.generation.recorded == 0)] | length' tremula-manifest.json)
    [ "$barren" = 0 ] || echo "::warning::tremula: $barren selected function(s) produced no mutants — this run is evidence of nothing"
    [ "$(jq '.selection.coverage' tremula-manifest.json)" != null ] || echo "::warning::tremula: selected without coverage — a mutant that survived may never be run at all"
```

> [!WARNING]
> **モデルへの依頼には費用がかかり、指定したファイルがそのプロバイダーに
> 送信されます。** ネットワーク呼び出しを行うのは`generate`、`triage`、
> そして`--github-pr`で投稿を求められた`tremula comment`だけです。
>
> **選定モードは自分で見つけたテストファイルも送信します。**
> `generate --diff-base`は対象ごとに1つのパスだけを見ます —
> 対象ファイルの上位でパッケージを宣言する最も近いディレクトリの
> `tests/test_<stem>.py` — そこで見つかったファイルは関数とともに読まれ、
> プロバイダーに送信されます。何が見つかったかはmanifestの
> `selection.functions[].inferred_tests`に記録されるため、何が送信されたかは
> 常に記録として残ります。
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
| プルリクエストに実行結果を報告する | [Comments](docs/01-architecture/comments.md) |
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
