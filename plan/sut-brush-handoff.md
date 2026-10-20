# SUT 브러시 작업 인수인계 (`codex/feature-sut-brush`)

작성: 2026-10-12, 갱신: 2026-10-15. 이 파일은 Git에 추적되어 있습니다.
최신 구현은 아래의 **텍스처 모드·사용자 필압 곡선 후속 구현**과 `docs/sut-analysis.md`를 확인하세요.

## 2026-10-12 초기 상태 (역사적 기록)

| 커밋 | 내용 |
|---|---|
| `96a3be54` | SUT 브러시 import (여러 팁 포함) |
| `623ffef1` | SUT 텍스처 매핑, 팁 참조 없는 브러시 → 원형 팁, 로컬 폴더 리포트 테스트 |

- 공개 무료 SUT 237개: **237개 import 성공, 텍스처 132개 적용, 패닉 0, 라이브러리 저장 왕복 237/237**
- 커밋되지 않은 변경: `crates/ui-egui/src/save_ui.rs`, `save_ui/tests.rs`, i18n `*.tsv`
  (SUT 작업과 무관, 이 작업 전부터 있던 것. 브랜치 전환 전에 stash/커밋 여부 결정 필요)
- 푸시 안 함

## 초기 연구 목록 (1번 모드 매핑은 후속 구현 완료)

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

## 2026-10-15 REA/Ghidra 후속 연구

- 현재 브랜치: `codex/feature-sut-brush`. 분석 보고서와 근거 위치는 `docs/sut-analysis.md`.
- 위 형식 메모의 소재 참조는 보완됨: 헤더 뒤 각 레코드에 BE u32 전체 길이와 BE u32 UTF-16LE 경로 바이트 길이가 있음. 부분 문자열 검색 대신 정확한 경로/순서/중복 참조를 처리하도록 수정.
- 위 `*Effector` 메모는 정정: bytes 4..8의 0x10만으로 활성 여부를 판단하면 안 됨. bytes 8..12의 활성 비트도 동시에 필요. 설치된 CSP 5.0.4의 직렬화/역직렬화/소비 함수에서 확인.
- Variant ID의 f32/클램프 비교를 제거하고 SQLite i64를 보존. 잘못된 TextureImage 타입은 브러시 전체 실패 대신 텍스처만 경고 후 제외.
- 텍스처 모드 숫자 0–9의 렌더 동작까지 후속 추적 완료. 내부 property 0x66 → config+0x14 → render object+0x48 → switch 연결. 0/4의 Normal/Outline 명칭만 다른 8개 매핑+리소스로 추론이며 수식은 직접 확인.
- 현재 Windows 로컬 SUT는 2개. 예전 237개 검증 숫자는 이번 변경 검증 결과가 아님.
- 원본 바이너리/소재/디컴파일은 `target/rea-tools/csp`에만 보관. 최종 검증 수치는 `log/devlog.md` 확인.

## 2026-10-15 텍스처 모드·사용자 필압 곡선 후속 구현

- `prism-paint::PressureCurve`: 2–16개 정규화 제어점, 2점 직선/3점 이상 2차 Bezier midpoint 연결. 네 effector의 독립 최소값·곡선 지원(size/opacity/flow/per-tip density). JSON `pressureCurve`, null은 선형으로 복귀.
- `crates/io/src/sut/effector.rs`: BE header/curve lengths/count/stride/f64 경계 검증. NaN/inf/순서/길이 오류는 경고 후 해당 dynamic만 해제. 지원·활성비트 모두 요구, 기타 활성 controller는 경고.
- `Texture.csp_mode`: JSON `cspMode`, None은 기존 Photoshop/ABR mode 유지. 10개 CSP 수식은 `paint/src/csp_texture.rs`; sampling→tone→density/mode, per-tip에서는 alpha를 먼저 반영, Height는 동일 alpha ceiling과 density×10.
- UI는 CSP mode dropdown과 ±100 톤 범위를 표시하고, preview/swatches 캐시에 mode 및 curve 포함. 커브 전용 GUI 편집기는 없으며 기존 `tools.setBrush` 명령으로 수정 가능.
- 근거: `ghidra-curves-1..5.txt`, `texture-pass1/2/5..9.txt`, `texture-findings.md`. REA native parser는 overlapping PE header를 거부하므로 direct Ghidra 근거로 구분. 실패한 texture pass3/4는 근거 아님.
- 남음: emphasize density/stress는 경고된 contrast 근사, rotation/tilt/speed/random 등, CSP fixed-point 및 누적/샘플링 pixel oracle. 로컬 2개 외 237개 모음은 여전히 없음.
- 최종 검증: paint 101, IO 115(1 ignored), engine 993(13 ignored) 통과. IO corpus 392 통과/2 ignored/43 suites. 로컬 2/2 SUT import·nonblank·정확한 저장 복원, panic_hunt 통과. UI 1374 통과/기존 Windows 저장 경로 3 실패/13 ignored; 새 CSP UI 테스트 2/2 통과. 오프스크린 한국어 10모드 메뉴 확인. Paint/IO strict scoped Clippy 통과, engine/UI는 기존 네 경고 종류를 명령행에서 제외한 검사 통과. Layers 29/0, rustfmt/diff 검사 통과. 상세 로그와 성능 수치는 docs/sut-analysis.md.

## 2026-10-15 Grainy Pen 작은 입자 후속 수정

### 최신 240% 두께 및 직접 비교

- 최신 실행 파일은 `target/fast/prism.exe` 2026-10-15 23:16 KST 빌드(67,658,752 bytes). 아래 20:32 빌드를 대체한다. SUT 재가져오기 필요.
- `cspTipScale:[1,2.4]`로 로컬 Y 축을 회전 전에 확장하며 영역/개수는 유지. 소스 네이티브 근거 및 안전 제한은 `docs/sut-analysis.md`의 240% 절 참조.
- 사용자 승인 후 CSP 새 투명 512×512 문서에서 15px/90% 마우스 직선을 직접 그리고 `target/rea-tools/csp/csp-live-15px-90.png`로 저장했다. 원래 사용자 문서는 보존했다.
- `thickness-live-comparison.png`는 실제 CSP/수정 전/후 비교. 중심 평균 알파 0.7169/0.3817/0.6481. 동일 랜덤·입력 스트림은 아니며 완전 일치하지 않는다. CSP의 옅은 가장자리, 최대 알파1(Prism0.902), 물가장자리 설정 및 누적 방식은 남은 조사 항목.
- 최종 검사 로그 접두사는 `target/thickness-`. 샌드박스 파일 경로 제한을 피하여 같은 최신 바이너리로 재검증: 엔진995 통과/13ignored, UI1375 통과/기존 저장 경로3실패/13ignored. 나머지 최종 결과는 devlog 최신 항목을 참조한다.

- 위 초기 메모의 마지막 PNG는 원본이 아닌 썸네일. ustar/C2F/부분 SQLite의 검증된 원본 링크를 따라 제한된 black/alpha 타일을 복원. Grainy 텍스처 300×300 대신 501×501, 팁 52×72. 미지원 원본은 명시적 경고 후 썸네일 대체.
- `cspSpray`: 영역 크기와 별도 2px 입자, step당 8개, bias -100 원주 배치. size/count 필압 곡선 독립. random 값만으로 활성화하지 않음. CSP 텍스처 alpha 극성 및 최대 두 단계 정수 mip/좌표 규칙 반영.
- `cspAntiAlias:false`: 원본 팁 nearest + fractional alpha 보존. Pencil의 50% threshold와 분리하여 작은 입자가 지워지는 문제 수정. AA가 켜진 단계는 기존 필터 근사로 남음. 근거 `spray-aa3..5.txt`와 주소는 `docs/sut-analysis.md`.
- 15px/90%/일정 pressure1 동일 Prism 경로 비교: `target/rea-tools/csp/grainy-matched-before.png`, `grainy-matched-after.png`. 작은 입자·거친 윤곽은 살아났으나 CSP 사용자 스크린샷보다 빈틈이 많음. 동일 native 경로 oracle이 아니며 완전 일치 주장 금지.
- 실행 빌드: `target/fast/prism.exe`, 2026-10-15 20:32 KST. SUT를 다시 가져온 뒤 15px/90% 지정. 기존 저장 프리셋은 자동으로 원본 소재/새 설정으로 바뀌지 않음.
- 디스크 부족으로 실패한 debug 테스트 링크는 캐시 정리 후 해소. `target/debug/incremental` 삭제로 약 54GB 확보했으며 이후 빌드가 일부 재생성. 소스/소재를 삭제하지 않음. 최종 검사 결과는 아래 및 devlog 확인.
- 후속 검증: paint 108, IO 127(2 ignored), 실제 Grainy 원본 raster oracle 1, engine 994(13 ignored), IO corpus 404(0 fail/3 ignored/43 suites), panic_hunt 1 통과. UI 1374 통과/기존 Windows 저장 경로 3 실패/13 ignored. Layers29/0, 기존 네 lint category만 명령행 제외한 관련 4개 crate Clippy 통과. `target/grainy-final-ui.png`의 15px/90% 및 synthetic spray 미리보기 육안 확인. 모든 상세 근거/로그는 `docs/sut-analysis.md`와 devlog.

## 2026-10-16 SUT 표시 패널 완료

- 현재 실행 빌드: `target/fast/prism-sut-display.exe` (02:40 KST, SHA256 `91e953f80e774e43fc0dae2e7868d0ab355b1a7b4080f32a0d177974d824b90d`). 실행 중인 기존 prism.exe는 보존. 새 빌드 실행 후 SUT 재가져오기 → 선택 → F5.
- 원본 표시 ID 목록/카테고리와 사용자 변경을 분리 저장하고 get/set/reset 엔진 명령 제공. 복제한 CSP 시험 브러시의 단일 변경 export 4개로 인코딩 확인. 슬라이더/단계는 공유 UI 설정으로 확인되어 소재 메타데이터에 저장하지 않음.
- 도구 설정/상세 설정, 눈 아이콘, 복원, 단위, 비활성/미지원 구분 구현. 실제 네이티브 화면에서 Grainy15px/90%, 두께240%, 텍스처100%/배율25% 및 다른 SUT 표시 목록 확인. 한국어/좁은 창/스크롤 확인.
- 검사: paint110, engine1000, IO132, UI1379 통과(기존 Windows 저장 경로 실패3개 재현 후 제외). panic_hunt/layers/parity/관련 Clippy 통과; corpus405 통과/0실패/4ignored/43suites. 자세한 로그와 마지막 UI 대상 검사 결과는 devlog 참고.
- 남음: 물 가장자리 등 미지원 렌더링, CSP 안티에일리어싱 강도 단계, 공유 단계표시 UI, 소재 카탈로그 텍스처 이름. CSP 픽셀 동일성은 주장하지 않음. 분석: docs/sut-analysis.md, 사용자 안내: docs/sut-brushes.md.

## 2026-10-16 물 가장자리(Watercolor edge) 구현 — Claude 세션

- 이전 세션이 03:06경 중단한 물 가장자리 REA/Ghidra 추적을 이어서 완료. 영구 Ghidra 프로젝트 `target/rea-tools/csp/projects/Persist`(-noanalysis 가져오기, 이후 -readOnly 실행) + `PrismFocused.java`(디컴파일 + `list 시작 끝` 선형 디스어셈블). 결과 `we6..we10.txt`, 비공개 요약 `water-edge-findings.md`, 공개 요약 `docs/sut-analysis.md` › Watercolor edge.
- 알고리즘: 덮인 픽셀마다 ±ceil(r) 정사각 창에서 빈 픽셀까지 유클리드 거리 d와 창 최대 커버리지 M → `f = d>1 ? (R-d)·10a/R : 3a`, `w = f·M·(0.95+0.0004·Bayer16)`를 알파에 더함(R=r+1, a=clamp(투명도 영향,0.01,1)). 명도 영향은 RGB 최대값 기준 어둡게. 드래그 후 처리 + 흐림이면 8비트 양을 정수 가우시안(반폭 trunc(blur), 끝 가중치 1/255)으로 흐리고 바깥 후광까지 그림. 디더 표는 표준 Bayer와 256/256 일치 → 생성식으로 독립 구현.
- 구현: `prism-paint::csp_water_edge`, `BrushSettings.cspWaterEdge`(Wet Edges 잠금과 함께), `StrokeRenderer::composite`(라이브: 더티 타일+반경 내 이웃 재합성, 드래그 후: 커밋 합성에만 적용, 흐림 필드 64Mi px 제한). SUT 임포터 매핑, 엔진 검증, F5 SUT 패널 1201 실제 동작(켜기/범위, 상세: 영향 2종/드래그 후/흐림), 한국어 라벨, 미리보기 캐시.
- 버그 수정: `brush_patch`가 직렬화에서 빠진 선택 필드를 null로 보내지 않아 UI에서 cspSpray/cspTipScale/cspWaterEdge 등을 끌 수 없었음 → null 전송, 열거형 변형 전환은 통째 교체 유지.
- 동시 작업 주의: 누군가(Codex 앱, 같은 브랜치)가 `Dynamic.csp_tilt`를 paint에 추가하던 중 io가 빌드 불가 상태였음. 빌드를 위해 `io/abr_map.rs`, `ui brush_panel_tests.rs`에 `csp_tilt: None`만 추가. 기울기 기능 자체는 손대지 않음.
- **핵심 남은 격차 = 기울기**: Grainy `BrushSizeEffector` 활성 0x30(필압+0x20), 50% 필드, 두 번째 곡선 (0,1)(0.5,1)(0.8,0)(1,0). 마우스(수직=1)에서 크기 50%. Prism을 7.5px로 그리면 CSP 실측과 지표 전부 일치(최대 1.000, 코어 평균 0.717/0.710, 범위 26/26, 단단한 행 248..259 동일, 희미한 픽셀 4319/4329). → SUT 임포터가 0x20을 `csp_tilt`로 매핑하면 Grainy 격차가 사실상 해소될 것으로 예상. 240% 두께는 맞음.
- 검사: paint 124, IO 133(3 ignored), IO 코퍼스 410/0 실패, 엔진 1002, panic_hunt 1, UI 1381(기존 Windows 저장 경로 3 실패), 4개 크레이트 clippy(기존 4개 lint 허용), layers 29/0, 한국어 F5 스냅샷 확인. 성능(36MP, 500점): 라이브 가장자리 53ms(없음 40ms), 드래그 후 블러 커밋 64ms(없음 9ms) — `water_edge_bench` 예제. 커밋/푸시 안 함.
- 남음: 기울기 매핑(진행 중 작업과 조율), 물 가장자리 스타일/강도/선명도와 색상 변형 경로, 캔버스 경계 처리 차이, 실제 앱에서 SUT 재가져오기 후 육안 확인.

## 2026-10-16 기울기(0x20) 임포터 매핑 — Claude 세션

- 사용자 허가로 Codex의 `CspTilt`를 이어받음. Codex 쪽은 03:06 이후 수정 없음, io에는 tilt 참조가 없었음.
- 네이티브 확정: 헤더 16 = A(응답 0 값 %), 40 = B(응답 1 값 %, 44바이트 미만 헤더면 100), 두 번째 곡선 = 기울기 곡선. 평가기 `값 *= A + (B−A)·곡선(입력)`을 필압 결과에 곱함. 0x40/0x80은 속도/랜덤(여전히 경고).
- `io/sut/effector.rs`가 필압·기울기를 각각 읽어 `Dynamic.csp_tilt`로 매핑(기울기 단독도 동적). 잘못된 기울기 값은 경고 후 기울기만 제외. `CspTilt::new`의 최소≤최대 제약 제거(원본도 순서 무관), `CspTilt`를 `prism_paint`에서 재노출.
- 결과: SUT 그대로 가져온 Grainy Pen이 CSP 실측과 모든 지표 일치(최대 1.000, 코어 평균 0.710/0.717, 범위 26/26, 단단한 행 248..259 동일, 희미한 픽셀 4329/4319). 비교 이미지 `target/rea-tools/csp/tilt-live-comparison.png`.
- 남음: 실제 펜의 기울기 입력 정의(현재 Codex 가정 1−hypot(tiltX,tiltY)/90) 네이티브 확인, 속도/랜덤 컨트롤, 물 가장자리 스타일·색상 변형, 캔버스 경계.

## 2026-10-16 손떨림 보정·속도에 따른 조정 — Claude 세션

- 속성: FlickerReduction 0x28, BySpeed 0x29, BySpeedType 0x2a, Stickness 0x32. 리더 `0x142b5b1b0` → `0x1429a7f30` → 입력 필터 세터 `0x141afbd40`.
- 필터 `0x141afb2e0`/`0x141afb5c0`: 원시 샘플(x,y,필압,ms) 링버퍼(1000) → 최근 W개(분수) 이동평균(위치+필압). 시작 W = S≤2면 S, 아니면 clamp(min(S,Stickness)>>1,2,15). 속도 조정 없음: W += min(0.5,(S−W+1)/S)→S. 속도 조정(유형0): W += (K−d)/K(양수면 ×0.5), K=2S+100, 최소 2, 최대 S. 1초 넘은 샘플 제외. 펜 업: 마지막 샘플을 floor(W)−Stickness회 재투입(따라잡기).
- 구현: `prism-paint::csp_stabilizer`, `BrushSettings.cspStabilizer`, `DabGenerator`에서 Smoothing 앞 단계, finish에서 따라잡기. 임포터 매핑, 엔진 검증(0..100), F5 패널 40(슬라이더)/41(체크박스).
- 미구현: 감속 시 조기 종료 상태기계(0x141afafe0), 다른 속도 유형 분기, 장치 유형 4 배율, 펜 업 샘플 필압. CSP 실측 비교 없음.

## 2026-10-16 입·출(시작과 끝) — Claude 세션

- 속성: 유형 0x488(0 길이/1 비율/2 페이드), 속도 0x489, 대상 0x48a, 입 0x492–0x494, 출 0x49c–0x49e. 대상 배열 [12,개수,12] + (효과기 ID, 사용, 최소%) 항목 → 평가기 0x100 제어 `값 *= m + (1−m)·계수`.
- 계수: 완성된 경로에서 시작 `sin(90°·s/입길이)`, 끝 `sin(90°·d/출길이)`. 비율은 경로 길이×비율, 속도 옵션은 처음/마지막 trunc(길이·0.04)개 입력 구간 길이로 제한, 합이 경로보다 길면 비례 축소.
- 구현: `prism-paint::csp_in_out`, `StepInput.distance`, `StrokeRenderer::with_full_path`(커밋 렌더·대칭·dabs), 그리는 중에는 길이 모드의 시작만. 대상: 크기·흐름·텍스처 농도·스프레이 크기/밀도. 임포터 매핑, 엔진 검증, 미리보기 캐시.
- 미구현/미검증: 페이드(유형2), 간격·두께·색 관련 대상, 길이 단위, SUT 패널 표시(ID 미확인). 로컬 SUT 2개 모두 꺼져 있어 CSP 실측 비교 없음.

## 2026-10-16 수채 색 혼합(물감 양·농도·색 늘이기) — Claude 세션

- 속성: 사용 0x43e, 유형 0x438, 물감 양 0x439(1081), 농도 0x43b(1083), 색 늘이기 0x43d(1085), 혼합 모드 0x43f, 효과기 0x43a/0x43c.
- 집기: 덥 래스터화 중 덥 영역 픽셀(레이어 위에 현재 스트로크 커버리지 합성)을 누적 → 다음 덥 전에 평균 알파/알파 가중 평균색, k=0.05/(늘이기+0.05)로 이전 색과 혼합(첫 샘플은 그대로).
- 덥: 불투명도 (1−d)·A+d, 색 (브러시·a²+집은색·(1−a²)·A)/(a²+(1−a²)·A). 첫 덥은 순수 브러시 색.
- 구현: `prism-paint::csp_mixing`, `Dab.paint_amount/paint_density`, `StrokeRenderer::with_mixing_source`(커밋·라이브·대칭), 임포터, 엔진 검증, F5 패널 1081/1083/1085 편집(혼합이 켜진 SUT일 때).
- 미구현: 유형 1/2(별도 샘플러), 지각적 혼합 모드, 색 변화 지터. 로컬 SUT는 혼합이 꺼져 있어 CSP 실측 비교 없음.

## 2026-10-16 색 혼합 CSP 실측 일치 + 자동 간격 — Claude 세션

- CSP 5.0.4에서 혼색 원형 브러시(크기 50, 물감 양 80, 농도 50, 늘이기 100, 자동 간격 3)로 빨강/노랑 줄 위에 그린 실측(`target/rea-tools/csp/csp-mix-live.png`)과 비교. Computer use는 Tablet PC 모드에서만 주입 드래그가 그려짐.
- 합성: 혼색 덥은 덮은 픽셀의 색과 알파를 (1−d)·A+d 목표로 lerp(알파를 낮출 수도 있음). 커버리지가 가득 차도 나중 덥이 이전 값을 덮어씀(`CoverageMap.paint_over`). 펜 다운 시 첫 덥 아래를 먼저 집음. 샘플은 누적 후에 취함.
- 자동 간격: `BrushAutoIntervalType`≠0이면 `BrushInterval` 무시. 0x1422e4830: 기본(1=0.12, 2=0.08, 3=0.04)×(3.5−2.5·max(경도,0.2))×(두께<1이면 두께), 지름 대비 비율. 이미지 팁은 경도 1. 유형 4/5는 경고 후 유형 2로.
- 결과: 스트로크 알파 램프 131→237 전 구간 ±2, 코어 평균 |Δα| 1.09, |ΔRGB|≈1. 비교 이미지 `target/rea-tools/csp/mix-live-comparison.png`.
- 남음: 둥근 팁이 CSP보다 양쪽 1px 넓음(실선 50 vs 48행), 유형 4/5 간격, 0x8000 플래그의 0.25 상한, 지각적 혼합 모드, 혼합 유형 1/2.

## 2026-10-16 CSP 안티에일리어싱 단계 + 남은 SUT 경고 정리 — Claude 세션

- 안티에일리어싱: `cspAntiAlias`를 0~3 단계로 확장(이전 bool은 false=0, true=1로 읽음). 근거 0x14264b860/0x1425260a0/0x1424b7aa0/0x142641180: 레벨 0은 픽셀 중심 nearest 1회, 그 외는 반 픽셀 간격 2×2 nearest 평균(합/4·255), 레벨 2는 표시 크기 이상인 mip, 레벨 3은 절반 크기 이상인 mip. 쌍선형 없음. 원형 팁은 정확한 모양(중간·강은 1/2px 상자로 근사). F5 패널 20번이 없음/약하게/중간/강하게 드롭다운.
- 색 혼합 집기 영역: CSP는 덥 사각형(quad) 안 모든 픽셀을 집음(투명 포함, 0x14264cb30). Prism `in_tip_quad`로 반영. 실측 램프 x=140까지 ±1, plateau 231~235(CSP 237).
- 지각적 혼색(BrushColorMixingMode 1): RGB(곡선 매개변수 0 = 선형) → sRGB 원색 XYZ → Hunt-Pointer-Estevez LMS/D65 → PQ(80/10000) 공간에서 평균. 집기·reservoir·물감 양 혼합 모두 적용. 경고 제거.
- BrushUseWaterColor2: 0x43e를 두 키에 같이 쓰는 사본. 둘 중 하나로 혼합 켬, 경고 제거.
- 손떨림 보정 속도 유형 1(레벨 30 이하): 최근 30개 샘플의 평균 이동 거리 v와 직진성 q로 목표 int(q·(2.5−v)·20), 그리는 중엔 최소 S, W를 입력당 0.25씩 이동. 경고 제거.
- 회색 소재(구아슈 팁): 픽셀 종류 17, 알파 평면 + 회색 평면. 블록 헤더 첫 값 = 펼친 타일 크기. 원본 그대로 읽음, 썸네일 경고 제거.
- 표시 ID 42 = 속성 0x2a(속도 조정 방식). F5 패널에서 편집 가능, 미인식 경고 제거.
- 사고: ko.tsv 편집 때 `\r\n` 기준으로만 나눠 LF 줄로 된 미커밋 번역 33줄이 한 덩어리로 지워졌음. `target/fast/prism.exe`(21:42 빌드)에 들어 있던 카탈로그에서 복원(쓰지 않는 문구 1줄 제외). ko.tsv는 CRLF/LF 혼재였음.
- 남음: CSP 원형 팁 경계(양쪽 0.72행, 폭 49.4) 및 plateau 237 차이 — 혼색 없는 원형 브러시 통제 실측 필요. RGBA 소재(종류 33), 간격 유형 4/5, 0x440 곡선 매개변수 속성 이름.

## 2026-10-16 거친 펜 가장자리 — Claude 세션

- 원인: `BrushRotationEffector` 131(=0x01 사용 | 0x80 랜덤)을 매핑하지 않아 두께 60% 타원 팁이 회전하지 않음. CSP는 덥마다 (r−0.5)·랜덤폭·360°로 회전해 가장자리가 거칠어짐.
- 근거: 0x1422e0db0 각도 계산(0x01 사용, 0x10/0x20 입력 +0x28 = 진행 방향, 0x40 입력 +0x48, 0x80 랜덤), 빌더 0x142b80a20(0x424/0x426/0x427).
- 수정: 임포터 `rotation()` — 0x30 → 진행 방향, 0x80 → 각도 지터 = 랜덤폭/2, 0x40 경고, 0x01 없으면 각도 0. 이전의 "3 = 진행 방향"은 근거 없는 매핑이라 제거(3은 기본 각도만).
- 결과: 거친 펜 렌더가 CSP처럼 거친 가장자리. Grainy Pen(스프레이)은 입자 각도를 따로 쓰므로 변화 없음. IO corpus 413/0/4.
- 남음: +0x48 입력(0x40) 정체, CSP와 같은 난수열은 아님.

## 2026-10-16 리얼 G 펜 원본 소재 — Claude 세션

- 경고 원인: 소재 SQLite 페이지 크기가 4096(기존 복구는 1024 가정) + 알파가 1비트 마스크(행 32바이트, 끝 단어 1, MSB 우선, 타일 8192바이트).
- 수정: `partial_material_pages_sized`(1024/4096, 첫 판독 페이지 = ceil(5120/크기)+1), 1비트 레이아웃 디코드. 썸네일과 비트 단위 일치. 리얼 G 펜·G펜·구아슈·거친 펜 모두 일반 경고만 남음.
- 남은 별개 경고: 세모네모펜 BrushSizeEffector 0x80(랜덤 크기), mixing-round 표시 ID 2개.

## 2026-10-17 Clip Studio 워크스페이스 모드 — Claude 세션

- 배경: SUT는 브러시뿐 아니라 채우기·선택·텍스트 등 기능 보조 도구도 담는다. 이전엔 전부 브러시로 들어옴.
- 판별: `SutToolKind::classify(input, output)` (`crates/paint/src/sut.rs`), 근거는 CSP 기본 `EditImageTool.todb`의 NodeInputOp/OutputOp/NodeIcon 표(`docs/sut-analysis.md` › Sub-tool operation IDs). 브러시 계열(10–14, 0/0)만 Brushes 패널·현재 브러시로 쓰임. 나머지는 라이브러리 보존 + 경고. 도트 펜은 1 px·AA 0.
- 모드: 설정 `workspace.style`(photoshop|clipStudio). Window › Workspace › Clip Studio → CSP 도구 Tools 창(`csp_tools.rs`) + 왼쪽 열 Sub Tool / Tool Property 패널(`sub_tool_panel.rs`). Photoshop 프리셋 선택 시 복귀. 도구별 선택 보조 도구 기억·저장, `ui.cspTool` 제어 op.
- 실행: 채우기(50/50) → Paint Bucket(참조 레이어·색 여유·인접·AA), 그러데이션 → Gradient(모양·디더·반전), 텍스트 → Type(크기·세로). 둘러싸 채우기(31/51)·말풍선·자 등은 회색 "미지원".
- 남음: 선택·도형 SUT 열 매핑 미확인(내장 보조 도구만 동작), 열 의미는 이름 기반 추정(코드 추적 아님), CSP에서 내보낸 실제 채우기/선택 SUT로 실물 확인 안 함.
