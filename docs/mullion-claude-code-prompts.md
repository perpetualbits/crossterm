# mullion — Claude Code Implementation Prompts

A sequenced series of prompts to drive the implementation described in
`mullion-design-note.md`. Each prompt is one Claude Code session (roughly one PR).
They are ordered; later prompts assume earlier ones have landed.

---

## How to use this

1. Put `mullion-design-note.md` in the mullion repo root (or `docs/`) so every
   session can read it.
2. Start each session by pasting the **Standing context** block below, then the
   numbered prompt for that phase.
3. One phase = one branch = one commit/PR. Don't let a session bleed into the next
   phase; the boundaries are where the design de-risks.
4. After each phase, confirm `aerie` still builds (`cargo build -p aerie` or from its
   own checkout) — the additive-only rule is a hard gate.

### Standing context (paste at the top of every session)

> You are working on **mullion**, a content-agnostic terminal UI tiling engine in
> Rust (GPL-3.0-or-later). Read `mullion-design-note.md` in full before writing code;
> it is the spec and its decisions are settled — do not relitigate them.
>
> Hard rules for every change:
> - **Additive only.** `aerie` (a shipping 1.0.x app) depends on mullion by path and
>   must keep compiling. Do not change existing public signatures; add new ones.
> - **Honor the test culture:** `TestBackend` for headless render assertions,
>   `proptest` with a checked-in regression corpus for invariants.
> - Match existing module conventions, naming, and error handling. Read the
>   neighbouring modules (`layout`, `border`, `table`, `tree`, `input`, `style`,
>   `buffer`) before adding to them.
> - Keep mullion content-agnostic: no domain semantics (no LDAP/DB/synth concepts).
> - Prefer small, reviewable commits within the branch. Write doc comments on every
>   new public item, with at least one example where non-obvious.
>
> Before coding, restate: (a) the public API you intend to add, (b) the invariants
> you'll property-test, (c) anything in the spec you find under-specified — ask
> rather than guess.

---

## Phase 1 — Floating tiles + free-space model (the foundation)

> Implement the **floating-tile placement pass** and the **free-space representation**
> from design-note §2.
>
> Add the ability for a tile to hold *floating children*: sub-tiles placed at a
> sub-rectangle inset from the parent's borders, leaving free space around them,
> without partitioning the parent. Floating children carry stable `TileId`s that
> survive a re-solve, exactly like tiling children.
>
> Deliver:
> - A way to declare floating children on a node and to solve their rectangles
>   alongside the existing tiling solve (existing tiling behavior unchanged when there
>   are no floating children).
> - A **free-space query** that, given the parent rect and the placed floating-child
>   rects, returns the leftover space as an *ordered, queryable structure*. It must
>   support two views without committing to either consumer's vocabulary:
>   - per-row free intervals (subtract child rects + a configurable gutter from a
>     row's width), for the future text engine;
>   - free-cell enumeration over a window, for the future router.
>   Make the query **viewport-bounded**: callers pass the visible row range / window;
>   never compute over an unbounded document or canvas.
> - Define the seek-shaped provider trait `RecordSource` from §4.1 in a new module now
>   (just the trait + a `Window` type + doc comments), so virtualization is not
>   retrofitted later. No implementations yet.
>
> Property tests: free-space intervals never overlap any floating child and always lie
> within the parent; gutters are respected; an empty floating-child set yields exactly
> the full row/window as one interval.
>
> Do **not** implement text, routing, or sockets in this phase. This is the load-
> bearing foundation; keep it minimal and correct.

---

## Phase 2 — Text engine core (bidi-aware, paginated, scrollable)

> Implement the **text engine core** from design-note §3 (excluding runaround, which
> is Phase 5).
>
> Pipeline, per paragraph → per visual line: UAX #14 line-break opportunities in
> logical order (`unicode-linebreak`); greedy width fill using grapheme-cluster widths
> (`unicode-segmentation` + `unicode-width`); UAX #9 BiDi reordering per visual line
> (`unicode-bidi`); emit cells in **visual** order (the terminal does not reorder).
>
> The bidi machinery must be present and correct from this commit even though the
> first visible milestone can be LTR. Specifically deliver:
> - A wrapped-text model that lays a paragraph out to a given width and produces visual
>   lines with cells in visual order.
> - The **logical↔visual cursor map** per visual line as a first-class output (§3.2):
>   a bijection enabling visual cursor movement over logical text, and coherent
>   selection across a direction boundary.
> - **Pagination** as a render mode: lay the model into fixed-height pages; and
>   **continuous scrolling** as the other view over the same model (§3.4).
> - Establish the chrome/content boundary (§3.3): chrome stays LTR; flowed content is
>   reordered. Route bidi through table cells and any label that flows, per the locked
>   decision in §6.3 — no bidi-correct/incorrect seam.
>
> Property tests: the cursor map is a bijection per visual line and round-trips to
> identity; no emitted glyph exceeds the target width; LTR-only input reorders to
> itself (identity) so the bidi path is exercised but provably inert on LTR.
>
> Add a `TestBackend` example rendering a short mixed-direction paragraph to lock
> visual output.

---

## Phase 3 — Row virtualization over the seek provider

> Implement **row virtualization** from design-note §4.1 on top of the `RecordSource`
> trait defined in Phase 1.
>
> Deliver a virtual list view that:
> - keeps only a window of rows materialized, fetching more via `fetch_after` /
>   `fetch_before` as the viewport moves (keyset/seek shape — never offset);
> - renders through the existing `Table`/`ColumnGrid` where appropriate (reuse, don't
>   reinvent);
> - drives a scrollbar whose thumb is **exact** when `exact_len` returns `Some` and an
>   **honest estimate** (via `approx_position`) otherwise — render the estimate
>   visibly as an estimate, per §6.2.
>
> Provide an in-memory `RecordSource` implementation for tests (a sorted keyed vector)
> so the windowing logic is testable without a real backend.
>
> Property tests: stitched windows are contiguous and gap-free (`fetch_after` then
> `fetch_before` across the same boundary agree); scrolling to the end and back
> materializes every row exactly once per pass; the window never exceeds its configured
> size.

---

## Phase 4 — Wrapped-line virtualization (huge flowed document)

> Implement **wrapped-line virtualization** from design-note §4.2, kept strictly
> separate from row virtualization (they share only the viewport abstraction).
>
> Deliver a viewer over one enormous flowed document that:
> - builds a lazy `byte-offset → wrapped-line` index incrementally as the user scrolls
>   or seeks, caches it, and invalidates it on width change;
> - supports scroll and seek without re-wrapping the whole document;
> - reuses the Phase 2 text engine for the actual wrapping of the visible window.
>
> Property tests: the index agrees with a brute-force full wrap on small documents;
> width change invalidates correctly; seeking to a byte offset lands on the correct
> wrapped line.

---

## Phase 5 — Runaround (slot-stream flow), then BiDi × runaround

> Implement **word-wrap runaround** from design-note §3.5 on the free-space model from
> Phase 1. **Land this in two stages within the branch; do not combine them until the
> first is proven.**
>
> Stage A (LTR runaround):
> - For each visible row, build the **slot stream**: free intervals after subtracting
>   floating-child rects + gutter (from the Phase 1 free-space query).
> - Flow wrapped tokens into slots instead of full-width lines. The obstacle-free case
>   must reduce to "one slot per row" through the *same* code path.
> - Reflow on floating-child drag must be viewport-bounded.
>
> Stage B (BiDi × runaround) — only after Stage A passes:
> - Make slot order within a row respect direction: flowing RTL content around an
>   exclusion reverses slot order within that row. Treat this as the known hazard it is
>   (§3.5) and test it explicitly.
>
> Property tests: no glyph is placed outside its slot; total flowed width per row ≤ sum
> of slot widths; with zero floating children, output is identical to Phase 2 flat
> wrapping (regression guard); RTL-around-exclusion produces the correct visual slot
> order on a hand-checked fixture.

---

## Phase 6 — Sockets / ports (lift the surf-field kernels)

> Implement **sockets** from design-note §5.1, lifting the proven geometry from the
> `spiral_stress` "surf field" (§7).
>
> Refactor — into a real mullion public API, not the demo's private `Painter` — the two
> kernels:
> - **Gap-interval geometry:** edge-gap computation, clamping to valid edge-local
>   indices (the `make_gap` logic), edge-local→absolute offset, robust at every box
>   size. Expose a socket as a `BorderGap` **with semantics**: `(side, offset,
>   direction, type)`, anchored (not autonomously animated).
> - **Connector-flow gradient:** the streaming-hue-along-a-gap effect (`stream_color`),
>   reusable to animate flow direction along a connector. Keep it optional/decorative
>   and parameterized.
>
> Drop the autonomous drift/pulse/split-merge motion — sockets are pinned, not
> wandering. Provide a `TestBackend` example placing sockets on the four sides of a
> tile at several sizes to lock glyph positions.
>
> Property tests: a socket's gap always lies within `1..len-1` for its edge at any
> tile size; sockets on the same edge never overlap when the API is asked to pack them.

---

## Phase 7 — Manual node placement (mouse + keyboard)

> Implement **manual placement** of nodes on a graph canvas, design-note §5.4 (manual
> half) and §5.7 (canvas concept), building on the floating-tile foundation.
>
> Deliver:
> - A **graph canvas**: a tile whose floating children are *nodes*, positioned in a
>   logical canvas coordinate space (which may exceed the tile — full pan/cull comes in
>   Phase 10; for now assume canvas == tile or larger with a fixed offset).
> - **Mouse placement:** drag a node to reposition; hit-test via the existing tile
>   hit-testing.
> - **Keyboard placement:** directional nudge and grid snap, via the existing
>   `InputRouter`/`Keymap` (reuse `vim_prefix` conventions where natural).
> - Nodes keep stable `TileId`s across re-solves; their positions are part of the
>   canvas state.
>
> No connectors yet (Phase 8) beyond the sockets from Phase 6 being placeable on nodes.
>
> Property tests: a placed node's rect stays within the canvas bounds (or is clamped
> per policy); keyboard nudge + inverse nudge returns to the original cell.

---

## Phase 8 — Orthogonal connector routing (grid A*, canvas space)

> Implement **orthogonal connector routing** from design-note §5.2.
>
> Deliver a router that, given sockets on nodes and the free-cell structure from
> Phase 1:
> - runs **grid A\*** over free cells with a **heavy bend penalty**, producing long
>   straight runs and few corners ("train tracks");
> - routes in **canvas space** (§6.5), so routes are stable under future scrolling and
>   recomputed on edits, not on camera motion;
> - renders connector cells using box-drawing, extending the existing junction glyph
>   logic to connector turns and to socket entry (the little ball-into-socket join).
>
> Per §5.2, do not attempt a hop-over glyph; leave crossing disambiguation to Phase 9
> (color/avoidance). Per §6.1, reroute every net per frame — at this scale that's fine.
>
> Property tests: a connector never occupies a node-interior or border cell; every
> connector path is orthogonal (only axis-aligned steps); endpoints coincide with the
> declared sockets.

---

## Phase 9 — Nudging + crossing/junction resolution

> Implement **nudging** and crossing resolution from design-note §5.3.
>
> Deliver:
> - When parallel connectors share a gutter, spread them onto **separate integer
>   tracks**, respecting **gutter capacity** (an N-cell gutter holds N parallels, no
>   more) — the router must fail gracefully or reroute when capacity is exceeded.
> - Extend junction glyph resolution to connector **crossings** and **T-joins**.
> - Implement **color-per-net** as the crossing-disambiguation strategy (§5.2);
>   optionally a crossing-avoidance bias in the router cost.
>
> Property tests: nudging never exceeds gutter capacity; two parallel connectors in a
> sufficient gutter never share a cell; crossing glyphs are chosen deterministically
> for a given pair of directions.

---

## Phase 10 — Graph viewport: 2D pan-and-cull

> Implement the **graph viewport** from design-note §5.7.
>
> Deliver:
> - A logical canvas larger than its tile, with a `(dx, dy)` **pan** offset moved by
>   keyboard (arrows / `hjkl`) and mouse (drag / wheel), in all four directions.
> - **Cull** rendering: draw only nodes and connectors intersecting the visible window
>   plus a margin.
> - **Exact** 2D scrollbars on both axes (the canvas bounding box is known) — contrast
>   the estimated row scrollbar; reuse the scrollbar widget with an exact length
>   source.
> - Confirm canvas-space routing makes tracks **stable under pan** (no crawling); add a
>   test that scrolling does not change any route.
>
> Property tests: culling never omits a node/connector that intersects the window;
> scrollbar thumb position is exact for a known canvas; routes are invariant under pan.

---

## Phase 11 — Semantic (level-of-detail) zoom

> Implement **semantic/LoD zoom** from design-note §5.6.
>
> Deliver:
> - **Discrete LoD thresholds** keyed on a tile's allocated area, swapping its renderer
>   along: collapsed → titled → ported → full-internal-graph.
> - **Continuous area animation** driving the tile's growth through the layout solver
>   by animating its constraints — generalize the technique demonstrated in
>   `spiral_stress`'s animated zoom (not the discrete `Tree::zoom_to` jump). Support a
>   zoom focus that is a tiling child, a **floating child**, or a **node inside a
>   nested graph**.
> - The two mechanisms cooperate: animate area continuously, cross LoD thresholds
>   discretely.
>
> Property tests: LoD selection is monotonic in area (more cells never selects a less
> detailed renderer); animated zoom converges to the target rect; focus targeting
> resolves to the correct tile/node id.

---

## Phase 12 — Sugiyama auto-layout

> Implement **layered (Sugiyama) auto-layout** from design-note §5.4 (automatic half).
>
> Deliver an auto-placement pass that:
> - assigns **layers** along the dataflow direction;
> - orders within layers by **median/barycenter** to reduce crossings;
> - **snaps to the grid** and writes node positions back into the canvas state (so
>   manual placement from Phase 7 and auto-layout share one position model);
> - for non-DAGs, breaks cycles first with a **feedback-arc-set** heuristic.
>
> Property tests: on a DAG, every edge points from a lower to a higher layer; crossing
> count after barycenter ordering is ≤ the count before; the pass is idempotent on an
> already-laid-out graph.

---

## Phase 13 — Nesting + taps (deep tail, optional v2)

> Implement the **deep tail** from design-note §5.5 only after Phases 1–12 are stable.
> Treat as two independent sub-projects; either may slip to a later release.
>
> - **Nesting:** a sub-tile that is itself a graph and a node in its parent, with inner
>   nodes wired to the parent's own sockets (group inputs/outputs, and taps). This is
>   **hierarchical layout with port constraints**; study ELK's approach before
>   designing the API.
> - **Taps / fan-out:** one output to many inputs over a shared trunk is a
>   **rectilinear Steiner tree**, not point-to-point routing; implement a Steiner
>   approximation rather than forcing it through the Phase 8 pairwise router.
>
> Property tests: a group's external sockets map consistently to inner endpoints; a
> Steiner fan-out connects all sinks with no orphan branch and respects orthogonality.

---

## Sequencing notes

- The two nonlinear-effort zones are **Phase 5 Stage B (BiDi × runaround)** and
  **Phase 13 (nesting × taps × routing)**. Give them their own sessions and don't
  rush them.
- After Phases 1–3 you already have enough to make `census` (the LDAP tool) real:
  virtualized user/group lists + bidi-correct editing. Consider building a `census`
  vertical slice there to validate the API in a real consumer before going deeper.
- After Phase 11 you have enough for the network/infra-overview and soft-synth use
  cases (nodes, routing, pan, semantic zoom, flow animation). Phases 12–13 are polish
  and scale for the largest graphs.
