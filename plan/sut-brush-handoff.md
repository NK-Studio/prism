# SUT 브러시 작업 인수인계 (`codex/feature-sut-brush`)

작성: 2026-10-12. 이 파일은 `plan/`(gitignore)에 있어서 브랜치를 바꿔도 그대로 남습니다.

## 현재 상태

| 커밋 | 내용 |
|---|---|
| `96a3be54` | SUT 브러시 import (여러 팁 포함) |
| `623ffef1` | SUT 텍스처 매핑, 팁 참조 없는 브러시 → 원형 팁, 로컬 폴더 리포트 테스트 |

- 공개 무료 SUT 237개: **237개 import 성공, 텍스처 132개 적용, 패닉 0, 라이브러리 저장 왕복 237/237**
- 커밋되지 않은 변경: `crates/ui-egui/src/save_ui.rs`, `save_ui/tests.rs`, i18n `*.tsv`
  (SUT 작업과 무관, 이 작업 전부터 있던 것. 브랜치 전환 전에 stash/커밋 여부 결정 필요)
- 푸시 안 함

## 다음에 할 일 (우선순위 순)

### 1. 텍스처 합성 모드 번호 확정 ← 가장 먼저

`TextureCompositeMode` 값이 CSP 메뉴 순서와 맞지 않아 지금은 전부 Multiply + 경고.
CSP에서 아래 브러시를 import → **보조 도구 상세 › 텍스처 › 텍스처 모드** 확인.

| 저장값 | 확인할 브러시 (ZER0COIL 팩) | 파일 수 | CSP 표시 (채우기) |
|---|---|---|---|
| 0 | `T07 WashPaper` | 9 | |
| 2 | `B01 CanvasFlat Brush` | 83 | |
| 4 | `E20 Dirt` | 10 | |
| 6 | `M21 Sketch Pencil` | 2 | |
| 7 | `T05 HatchFibre` | 4 | |
| 9 | `M28 Old Ink Brush` | 24 | |

CSP 메뉴(UI 순서): 보통, 곱하기, 빼기, 비교, 윤곽선, 오버레이, 색상 닷지, 하드 믹스, 높이.
Prism `MaskMode`: Multiply, Subtract, Darken, Overlay, ColorDodge, ColorBurn, LinearBurn, HardMix, LinearHeight, Height.

- 반영 위치: `crates/io/src/sut/mod.rs`의 `TEXTURE_MODES` 테이블
- 대응 없는 모드(보통, 비교, 윤곽선)는 가까운 모드 + 경고로 처리할지 결정
- `maps_texture_material_and_settings` 테스트에 모드 assert 추가

### 2. 근사 처리 개선 (텍스처)

- **농도 강조**(`TextureStressDensity`, 80개 파일): 지금은 대비 +0.5로 추정. CSP와 비교 후 조정
- **필압 농도 + 스트로크 단위 적용**(39개 파일): 엔진의 필압 깊이 제어가 "각 팁에 적용"일 때만 동작해서 무시 중
- **텍스처 회전**(9개 파일): 엔진 `Texture`에 회전 필드 없음 → `prism-paint` 확장 필요
  (확장 시 `prism-format` manifest/convert에 필드 추가, AGENTS.md §8)

### 3. 아직 지원하지 않는 기능 (237개 기준 파일 수)

| 기능 | 파일 수 | 메모 |
|---|---|---|
| 수채 `BrushUseWaterColor` | 96 | 물감 혼합. Mixer Brush(`paint.mixerBrush`) 재사용 가능성 검토 |
| 입·출 페이드 `BrushUseIn/Out` | 28 | 엔진 Fade 제어(`Control::Fade`)로 매핑 가능해 보임. 쉬운 편 |
| 수채 경계 `BrushUseWaterEdge` | 14 | |
| 팁 순서(랜덤 등) | 14 | 지금은 순환 순서 |
| 듀얼 브러시 `UseDualBrush` | 13 | 엔진에 `DualBrush` 있음. ABR 매핑 참고 |
| 스프레이 `BrushUseSpray` | 5 | 엔진 Scattering으로 근사 가능 |
| 리본 `BrushRibbon` | 2 | |

### 4. 그 밖에

- 서브 도구가 여러 개 든 SUT는 첫 번째만 import
- 렌더링이 CSP와 같은지는 미검증 (테스트는 "깨지지 않고 그려진다"까지만 확인)

## 테스트 방법

### 수집한 브러시 (로컬, 커밋 금지)

`~/Downloads` 아래:
- `5x free brushes` (ed-of-all, itch.io)
- `ZellkBrushes - SmallPack - 2018` (itch.io)
- `Yin Cel Animation Brushes/Yin Cel Animation Brushes` (itch.io)
- `ZER0COIL Clip Studio Brushes` (itch.io, 218개)
- `세모네모펜.sut` (최초 oracle)

### 폴더 리포트 실행

한 폴더에 모으기 (심볼릭 링크):

```sh
D=/tmp/sut-collected; mkdir -p $D; cd ~/Downloads
ln -sfn "$PWD/5x free brushes" $D/ed-of-all
ln -sfn "$PWD/ZellkBrushes - SmallPack - 2018" $D/zellk
ln -sfn "$PWD/Yin Cel Animation Brushes/Yin Cel Animation Brushes" $D/yin
ln -sfn "$PWD/ZER0COIL Clip Studio Brushes" $D/zer0coil
```

```sh
PRISM_TEST_SUT_DIR=/tmp/sut-collected PRISM_TEST_SUT_REPORT=/tmp/sut-report.md \
  cargo test -p prism-engine --lib local_sut_folder_report -- --ignored --nocapture
```

리포트: 파일별 상태 / 팁 / 텍스처 / 크기 / 칠해진 픽셀 / 경고 수 / 시간 + 경고 빈도표.
패닉이 있을 때만 테스트 실패.

### 단일 oracle

```sh
PRISM_TEST_SUT=~/Downloads/세모네모펜.sut cargo test -p prism-engine local_sut_import -- --ignored
```

### 텍스처 비교 이미지 (왼쪽 텍스처 켬 / 오른쪽 끔)

`prism-cli run --new ... --cmd brush.presets.importSut --cmd tools.setBrush {"size":60} --cmd paint.stroke ... --cmd tools.setBrush {"texture":{"enabled":false}} --cmd paint.stroke ... --out compare.png`

## 코드 위치

- 리더: `crates/io/src/sut/mod.rs` (`read_sut`, `texture`, `referenced`, `TEXTURE_MODES`)
- SQLite 파서: `crates/io/src/sut/sqlite.rs`
- 합성 테스트: `crates/io/src/sut/tests.rs` (`fixture_with(tip_refs, texture)`)
- 명령/로컬 테스트: `crates/engine/src/sut_import_cmds.rs`
- 문서: `docs/sut-brushes.md`
- 참고 매핑: `crates/io/src/abr_map.rs` (ABR 텍스처/듀얼 브러시)

## SUT 형식 메모 (관측)

- `Node.NodeVariantID` → `Variant.VariantID` 행이 현재 설정
- 팁/텍스처 참조 blob: `u32 BE 헤더크기(8)`, `u32 BE 개수`, 이어서 UTF-16LE 소재 경로
  → `MaterialFile.OriginalPath`와 매칭, `FileData` 안의 마지막 PNG가 이미지
- `*Effector` blob: 바이트 4..8의 bit `0x10` = 필압 사용
- `BrushUsePatternImage=1`인데 참조가 NULL이고 `MaterialFile` 테이블도 없는 파일 있음 → 원형 팁
- 텍스처 값 범위(관측): 밝기 -19..37, 대비 -33..74 (UI ±100 가정), `TextureScale2`는 % (1..597)

## 마무리 체크리스트 (AGENTS.md §5)

```sh
cargo test -p prism-io -p prism-engine
cargo clippy -p prism-io -p prism-engine --all-targets -- -D warnings
cargo xtask layers
```

devlog(`log/devlog.md`)에 결과 숫자를 기록하세요.
