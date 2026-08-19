# tremula

[English](README.md) | **한국어** | [日本語](README.ja.md)

**코드에 그럴듯한 버그를 심고 — 테스트가 흔들리는지 확인하세요.**

tremula는 외부에서 정의된 뮤턴트를 테스트 스위트에 대해 실행하고 어떤
뮤턴트가 살아남는지 보고합니다. 살아남은 뮤턴트는 테스트가 커버하지 못하는
빈틈의 증거입니다.

> [!NOTE]
> 초기 개발 단계입니다. 패키지는 아직 배포되지 않았으며 인터페이스가 바뀔 수
> 있습니다.

## 왜 tremula인가

- **무엇을 변이할지는 사용자가 정합니다.** manifest가 파일, 정확한 바이트
  범위, 교체할 내용을 지정합니다. 직접 작성하거나 모델이 제안하게 할 수
  있습니다. 아무도 요청하지 않은 뮤턴트를 쏟아내는 패턴 스캐너가 아닙니다.
- **"killed"의 정의는 하나입니다.** Rust core가 중립 신호로 모든 판정을
  내리고, 언어 팩은 스위트를 실행하고 일어난 일만 보고합니다. 두 언어가
  탐지의 의미를 다르게 정의할 일은 없습니다.
- **survivor는 모델의 말이 아니라 실험으로 분류합니다.** `tremula triage`는
  survivor를 구분할 입력을 모델에 물어본 뒤, 그 입력을 함수의 두 버전에 대해
  실행하고 각각이 한 일을 보고합니다.
- **넘겨줄 수 있는 증거를 만듭니다.** `tremula bundle`은 실행의 report,
  patch, 로그를 해시 인덱스가 붙은 디렉터리로 패키징해, 동료나 코딩 에이전트가
  재현할 수 있게 합니다.

## 소스에서 설치

tremula는 Python 언어 팩과 Cosmic Ray를 통해 Python 3.10 이상을 지원합니다.
두 패키지 모두 아직 PyPI에 없으므로, 이 저장소를 클론해 측정하려는 프로젝트에
함께 설치하세요:

```sh
uv add --dev --editable /path/to/tremula /path/to/tremula/packs/python
uv run tremula --version
```

이렇게 하면 CLI와 언어 팩이 테스트 스위트와 같은 프로젝트 환경에 놓입니다.

## 빠른 시작

실행하려면 무엇을 변이할지 적은 manifest가 필요합니다. 직접 작성하거나 모델에
요청한 뒤, 검증하고 실행하세요:

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

`0`은 살아남은 뮤턴트가 없다는 뜻이고, `1`은 있다는 뜻이며, `2`는 실행
결과를 신뢰할 수 없다는 뜻입니다. 살아남은 것이 있으면 읽기 전에 분류하고,
증거를 패키징하세요:

```sh
uv run tremula triage --model gpt-5.2-2025-12-11
uv run tremula bundle
```

## 풀 리퀘스트가 바꾼 것만 변이시키기

함수를 직접 지목하는 대신, 비교할 리비전을 알려주면 `generate`가 대상을 스스로
정합니다. 테스트 실행의 커버리지까지 주면, 바뀐 라인 중 테스트가 실제로 지나가는
곳만 변이시킵니다:

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

비교의 기준은 브랜치가 베이스에서 갈라진 지점이므로, 베이스가 그 뒤에 나아갔더라도
베이스 자신의 커밋이 이 변경에 섞이지 않습니다. `--max-functions` 기본값은 5이고,
상한에 걸려 제외된 함수는 조용히 버려지지 않고 기록됩니다. `--coverage` 없이 돌리면
바뀐 함수 전부가 대상이 되며, 출력이 그 사실을 밝힙니다 — 살아남은 뮤턴트가
"테스트가 지나가지 않아서" 살아남았을 수 있기 때문입니다.

**변이시킬 것이 하나도 없는 변경은 실패가 아니라 성공입니다.** `0`으로 끝나고
manifest도 그대로 기록합니다. 바뀐 라인 중 어느 테스트도 지나가지 않는 곳이
어디인지가, 그런 실행이 내놓는 가장 값진 정보이기 때문입니다. `run`은 그런
manifest에 대해 리포트만 쓰고 끝내므로, CI 잡은 분기 없이 명령을 늘어놓으면 됩니다.

각 함수가 왜 선정되었는지는 manifest의 `selection`에 기록되고, 거기서 실행
디렉토리와 증거 번들까지 그대로 실려 갑니다. `tremula comment`는 manifest와 실행
디렉토리를 읽어 풀 리퀘스트 코멘트 본문을 출력하며, `--github-pr <N>`을 주면
직접 게시합니다 — 스레드에 쌓는 대신 자신이 이전에 남긴 코멘트를 교체합니다.
자세한 내용은 [Comments](docs/01-architecture/comments.md)를 보세요.

> [!WARNING]
> **모델 호출은 비용이 들고 지정한 파일이 해당 제공자에게 전송됩니다.**
> 네트워크 호출을 하는 명령은 `generate`, `triage`, 그리고 `--github-pr`로 게시를
> 요청받은 `tremula comment`입니다.
>
> **선정 모드는 스스로 찾은 테스트 파일도 전송합니다.** `generate --diff-base`는
> 대상마다 단 하나의 경로만 봅니다 — 대상 파일 위쪽에서 패키지를 선언하는 가장
> 가까운 디렉토리의 `tests/test_<stem>.py` — 거기서 찾은 파일은 함수와 함께 읽혀
> 제공자에게 전송됩니다. 무엇을 찾았는지는 manifest의 `selection.functions[].inferred_tests`에
> 기록되므로, 무엇이 전송되었는지는 항상 기록으로 남습니다.
>
> **manifest는 곧 실행할 코드입니다.** 모든 `replacement`는 테스트 스위트의
> 일부로 실행되므로, 신뢰하는 manifest만 실행하세요.
>
> **뮤턴트는 소스에 직접 적용됩니다.** 실행이 강제 종료되면 뮤턴트가 소스에
> 남을 수 있습니다. `tremula restore`가 실행 스냅샷에서 되돌려 주며, 소스가
> 변경된 채로 실패한 실행은 정확한 복구 명령을 출력합니다.

프로젝트의 `.gitignore`에 `.tremula/`와 `tremula-bundle-*`를 추가하세요. 실행
결과는 프로젝트 안에 기록되며, 커밋하면 이후 모든 실행이 변경된 작업 트리를
보게 됩니다.

## 다음으로 볼 것

| 작업 | 가이드 |
| --- | --- |
| 복구까지 포함해 첫 실행 완료하기 | [첫 뮤테이션 테스트 실행](docs/guides/first-run.md) |
| triage 결과 이해하고 survivor dismiss하기 | [survivor 검토와 dismiss](docs/guides/survivor-review.md) |
| 동료나 코딩 에이전트에게 실행 결과 패키징하기 | [증거 번들 공유](docs/guides/bundle-sharing.md) |
| 풀 리퀘스트에 실행 결과 보고하기 | [Comments](docs/01-architecture/comments.md) |
| 아키텍처, 계약, 결정 기록 | [문서 인덱스](docs/README.md) |

생성된 JSON Schema와 공유 예제는 [`contracts/`](contracts/README.md)에
있습니다. 링크된 문서는 영어로 작성되어 있습니다.

## 개발

저장소를 설정하고 CI에서 사용하는 것과 같은 검사를 실행하세요:

```sh
uv sync
just lint
just test
just e2e
```

Rust 계약 타입을 변경했다면 `just contracts`를 실행하세요. `just build`는
배포용 wheel을 `dist/`에 만듭니다.

## 라이선스

MIT
