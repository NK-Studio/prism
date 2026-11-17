# Content-Aware Fill investigation

Started 2026-11-17 (maintainer request). Edit › Content-Aware Fill… was rebuilt as Photoshop
2026's Content-Aware Fill workspace, with the fill following the structure in the maintainer's
research notes on Photoshop 2026 (27.4, macOS arm64) `AdbePM.framework`. This is not a claim of
Photoshop source recovery.

## Authorization and boundary

On 2026-11-17 the maintainer asked to rebuild Content-Aware Fill using their research notes
(outside this repository: `app/MECHANICS.md` §7, `app/mechanics.json`, `app/src/inpaint.cpp`)
and screenshots of Photoshop's own workspace, taken for each feature. The same boundary as the
other investigations applies: Prism's code is written independently from the documented rules
and constants, never from pseudocode; raw analysis, binaries and Photoshop screenshots stay
outside source control (the screenshots used are in the local, gitignored `plan/content-aware-fill/`).

## Workspace (observed in Photoshop 27.4, Korean UI)

| Part | Photoshop | Prism (`crates/ui-egui/src/caf_ui.rs`) |
|---|---|---|
| Layout | options bar on top; tools on the left; document; Preview panel; Content-Aware Fill panel | same, full window |
| Tools | Sampling Brush, Lasso (with flyout), Hand, Zoom | Sampling Brush (B), Lasso (L), Hand (H), Zoom (Z); no polygonal lasso yet |
| Sampling Brush options | ⊕ / ⊖ mode, Size (56 seen) | same; Alt flips the mode |
| Brush mode default | ⊖ in Auto and Rectangular, ⊕ after choosing Custom | same |
| Lasso options | new / add / subtract / intersect, Feather, Anti-alias, Expand, Contract, unit (3 px) | modes, Expand, Contract, step 1–50 px; **no Feather / Anti-alias fields yet** |
| Sampling Area Overlay | Show Sampling Area ✓, Opacity 60 %, colour (green), Show: Sampling Area / Excluded Area | same |
| Sampling Area Options | Auto / Rectangular / Custom, Sample All Layers (disabled with one layer); section reset | same |
| Custom | starts empty, Preview shows the hole unfilled | same (`previewEmpty`) |
| Brushing in Auto | the area becomes Custom starting from the Auto area | the panel shows Custom; params keep `sampling: auto` + `samplingStrokes` |
| Fill Settings | Color Adaptation 없음/기본값/고/최고값; Rotation Adaptation 없음/저/중/고/전체; Scale; Mirror; section reset | same labels (`caf` i18n context) |
| Output Settings | Output To: 현재 레이어 / 새 레이어 / 레이어 복제 (New Layer shown) | same, New Layer by default |
| Footer | ↺ reset, Cancel, Apply, OK | same; Apply fills and keeps the workspace open |
| Preview | own zoom field and slider (131 % seen) | same, framed on the fill area |
| Tooltips | Rectangular: "채우기 영역 주위에 사각형 영역 사용" | — |

## Fill (`crates/algo/src/caf.rs`)

| Part | Research notes | Prism | Status |
|---|---|---|---|
| Search bounds (`ProposeSearchBounds`) | centred on the hole, area `factor² × hole area`, sides ≥ 2 × the hole's, pushed inside the image; base size from 50 px in 5 px steps below 175 | `search_bounds_with`: as written; the base grows while the window holds fewer sampleable pixels than the hole's area | rule as written; growth condition estimated |
| Auto factor | crop factor 4.0 set by the UXP engine path (`0x106602988`) | 4 | from the notes |
| Rectangular | not separated in the notes | search bounds, factor 3 (`factor` param 1–10, or `margin`) | estimate |
| Auto exclusion | not in the notes; Photoshop's overlay leaves out dark cloud blobs | colours far from every k-means cluster (k ≤ 4) of the hole's border ring are left out, then an opening by half a patch | estimate from observation |
| Pyramid | `MipMapInterface`, `-min_coarse_size` | halve until the hole's long side ≤ 3 patches or the level < 4 patches | estimate (value not found) |
| Coarsest guess | — | onion peel of the border mean | as the notes' app |
| Search | PatchMatch (`SearchBody`, Threefry RNG) | propagation + shrinking random search, row bands in parallel (SplitMix per band, deterministic) | structure as written |
| Vote | `VoteBody`; the notes' app weighs `exp(−d / 2σ²)`, σ² the 75th percentile of patch costs (Wexler 2007), the finest level's last vote the 25th | as the app | percentiles estimated |
| Level transition | vote with the inherited matches before searching (searching against the blurry upsampled guess loses the texture) | same | from the notes' app (wallet photo: grain kept) |
| EM iterations | `-em_iters`, `-em_iters_final` (values not found) | 4 per level, half on the finest | estimate |
| Patch size | not found | 7 px | estimate |
| Mirror | `$$$/PatchMatchPanel/Name/Mirror` | flipped source patches in the search | structure |
| Colour adaptation | brightness/contrast adaptation (+ Prism's membrane pull, see below) | per-match channel offset, the clamped mean difference over the target patch's **known** pixels (none under a quarter known); limits 0 / 0.06 / 0.15 / 0.3 | limits estimated; known-only is Prism's choice (the app fits all pixels, which holds the fill at the blurry guess's brightness) |
| Rotation / scale adaptation | `-gpm_min_scale/-gpm_max_scale` | rotated and rescaled copies of the sampling area beside the image (Prism's earlier approach) | not from the notes |
| Not enough patches | `ExceptionNotEnoughPatches` | coarse levels without sources keep the estimate; none at the finest is "nothing to copy from" | — |

The finer levels refine the inherited field with a local random search after their first EM
iteration (a Prism performance choice, not from the notes).

## Card photo test (2026-11-17)

The maintainer's own photo (3024 × 4032, a card wallet on wood; not in the repository), rectangle
690,1400 1660 × 1390, defaults. A local lab test (`plan/content-aware-fill/caf_lab.rs`) measures
the fill against a 60 px ring around it: mean luminance gap, high-frequency energy ratio (mean
|Laplacian|) and the share of 16 × 16 blocks with less than half the ring's median spread.

| Version | luminance gap | detail ratio | smooth blocks |
|---|---|---|---|
| first workspace build (vote `1/(cost/area+1e-3)`, search on the upsampled guess) | wallet-shaped light ghost | — | — |
| notes' update (vote first, exp weights, factor 4) | +0.033 | 0.93 | 19.6 % |
| winner-takes-all final vote / all votes sharp | +0.020 … +0.033 | 0.92–0.96 | 17–20 % |
| Auto also leaving out low/high detail (0.5× / 2.5× the ring's) | −0.028 | 0.77 | 29 % (worse; dropped) |
| **+ low-frequency pull to the border membrane** (default 0.65, very high 1.0) | +0.010 / −0.002 | 0.94 | 19 % |

The hole is half the picture wide, so the factor-4 search window is the whole photo, and the
sampling area itself has 21 % smooth (out-of-focus) blocks: the fill reproduces the sampling
area's statistics, while the ring right around the wallet has denser grain. The membrane pull
(Prism's choice, mapped to Color Adaptation: none 0, default 0.65, high 0.85, very high 1.0) is
what removed the visible rectangle; the blotches stay where the sampling area has them (paint
them out of the sampling area with the Sampling Brush, as in Photoshop).

## Performance (2026-11-17, M2 Pro, release `perf_scenarios`)

P21 Content-Aware Fill 10 MP (200 px hole): p50 150 ms (budget 1500). P49 cancel on 24 MP:
max 51 ms (budget 200; 588 ms before cancellation checks were added to the mask building, the
initial copies, the re-costing and the colour offsets, and the Auto colour test ran in parallel).

## Unknowns

- Photoshop's exact Auto exclusion rule and how Rectangular sizes its rectangle.
- Patch size, EM iteration counts, the coarsest size and colour-adaptation strengths.
- Whether Photoshop blends the fill's border: `PM_Composite` only mixes partly selected pixels, so a hard selection keeps a hard edge; the lighter, blotchy fill on large holes is still open.
- `minCoarseSizes()`: Photoshop runs one final fill per minimum coarse size; the list's values are set at run time.
- Lasso Feather and Anti-alias in the workspace (not wired yet).
