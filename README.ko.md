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

> [!WARNING]
> **모델 호출은 비용이 들고 지정한 파일이 해당 제공자에게 전송됩니다.**
> 네트워크 호출을 하는 명령은 `generate`와 `triage`뿐입니다.
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
