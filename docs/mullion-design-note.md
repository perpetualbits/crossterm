# mullion — Design Note: Floating Tiles, Text Engine, and Node Graphs

Status: design spec, pre-implementation
Audience: mullion maintainers and Claude Code implementation sessions
Scope: the next major capability layer on top of mullion 0.3.x

---

## 1. Thesis

mullion today is a content-agnostic **tiling engine**: you describe a layout as a
tree of tiles, the solver hands back one `Rect` per tile, and you paint into the
rectangles. Identity attaches to a `TileId` derived from durable domain identity,
so a layout can churn at runtime without losing focus, scroll, or zoom state.

This note extends that engine along one axis: **structured, navigable, text-rich
spatial interfaces**. Concretely, three new capabilities that turn out to share a
single foundation:

1. A **text engine** — input and output, bidirectional from day one, with
   first-class pagination, scrolling, and word-wrap that can flow around obstacles.
2. **Virtualized data** — scroll smoothly through a million LDAP or SQL records
   without materializing them.
3. **Node graphs** — floating sub-tiles connected by orthogonal connectors that
   plug into edge sockets, placeable by mouse or keyboard, with auto-layout and
   auto-routing, nestable to arbitrary depth.

The unifying claim is that all three are consumers of **one** new abstraction —
the *floating tile and the free space around it* — and that building that
foundation once, cleanly, is what keeps the three features from drifting into
three incompatible subsystems.

### Why this set of features (the generality test)

The features are chosen so that mullion becomes the substrate for a wide class of
terminal applications, not a single app's toolkit. The same engine should carry:

- **LDAP administration** (the `census` project): virtualized user/group lists,
  bidi-correct attribute editing, paginated detail views.
- **Database design**: entity tiles with column lists, relationships as connectors,
  auto-layout of a schema.
- **Node-graph editors** generally: any boxes-and-wires interface.
- **A text-based soft-synth**: oscillator/filter/envelope modules as nodes, patch
  cords as connectors, signal direction shown by flow animation along the wire.
- **Network and infrastructure overviews**: physical topology, VLAN layers,
  Nomad/Consul/Vault service structure (e.g. in LOFAR) — large graphs navigated by
  pan and by *semantic zoom* across layers of detail.
- **A text-based IDE** that wires snippets of assembly to one another: code blocks
  as nodes, data/control flow as connectors.

If one abstraction serves all of these, it is the right abstraction. Each feature
below notes which of these use cases it unlocks.

---

## 2. The foundation: floating tiles + free space

Today every tile *partitions* its parent — splits consume the whole parent with no
leftover. The new primitive is a child that occupies a **sub-rectangle inset from
the parent's borders**, leaving free space around it on at least one side, possibly
several.

A parent that holds floating children must expose two things:

- the **placed rectangles** of its floating children (for drawing them and for
  hit-testing), and
- the **free space** between and around them, as an *ordered, queryable structure*
  rather than an afterthought.

That free-space structure is the load-bearing output. Two different consumers read
it two different ways:

- The **text engine** reads it as *line slots*: for each visible row, the free
  intervals left after subtracting floating-child rectangles (plus a gutter).
- The **node-graph router** reads it as *routing channels*: the free cells through
  which orthogonal connectors may run.

Because both consumers derive from the same structure, runaround text and connector
routing never disagree about where the obstacles are. **Build the floating-tile
placement pass and the free-space representation first, alone, and get them clean.**
Everything else inherits their quality.

Design obligations of the foundation:

- Floating children carry stable `TileId`s exactly as tiling children do, so their
  focus/scroll/zoom/placement survive a re-solve.
- The free-space query is viewport-bounded: callers ask about the *visible* rows or
  the *visible* canvas window, never the whole document or canvas.
- The seek-shaped data-provider trait (Section 4) is defined *here*, alongside the
  foundation, so virtualization is not retrofitted later.

Additive-only constraint: `aerie` (shipping at 1.0.3) depends on mullion by path and
must keep compiling. Every addition in this note is purely additive; existing tiling
behavior is unchanged when no floating children are present.

---

## 3. The text engine

Unlocks: bidi attribute editing and paginated detail views (LDAP); code-block bodies
(assembly IDE); any prose or label rendering across the whole engine.

### 3.1 Bidirectional from day one

Retrofitting BiDi is the regret everyone reports, because direction leaks into cursor
movement, selection, wrapping, and width math — bolt it on later and you touch all of
them twice. So the pipeline is bidi-aware from the first commit, even while the first
visible milestone only exercises LTR.

Pipeline, per paragraph → per visual line:

1. **Line-break opportunities** in *logical* order (UAX #14, `unicode-linebreak`).
2. **Greedy fill** of the available width (or slot; see runaround) using
   grapheme-cluster widths (`unicode-segmentation` + `unicode-width`, already deps).
3. **BiDi reordering** per visual line (UAX #9, `unicode-bidi`): resolve embedding
   levels, reorder runs to visual order.
4. **Emit cells in visual order.** Do not trust the terminal to reorder — most
   emulators render cells in memory order, so the engine must hand them over already
   visually ordered.

### 3.2 The logical↔visual cursor map

The piece people underestimate. Arrow-right moves *visually* but edits *logically*,
so each visual line needs an index translation between the two orders. The same map
is what makes a selection that crosses a direction boundary coherent rather than
garbled. Treat it as a first-class output of the layout pass, not a derived
convenience.

### 3.3 The chrome/content boundary

Borders, junctions, table rules, and other chrome stay LTR. Only *flowed content* is
reordered. Decide this boundary explicitly and once; leaving it implicit is what
later corrupts node labels and table cells. Per the cross-cutting decision in
Section 6, BiDi reaches **everything that flows** — prose bodies, table cells, and
node labels — so the engine has no seam where one path is bidi-correct and another is
not.

### 3.4 Pagination and scrolling as first-class citizens

Pagination is not a print-time afterthought; it is a render mode. The engine must be
able to lay a body out into fixed-height pages (with widow/orphan awareness as a
later refinement) and to scroll continuously. Both are views over the same wrapped
model. See Section 4 for the virtualization that lets this scale.

### 3.5 Word-wrap with runaround (slots)

The elegant framing is a **slot stream**. For each visible row, subtract every
floating child's rectangle (plus gutter) from the row's width, yielding 1..n free
intervals — "left of tile", "right of tile", or both. Flatten the visible rows into
an ordered stream of slots and flow wrapped tokens into *slots* instead of into
full-width lines.

The plain, obstacle-free case is simply "every row is one slot", so a single code
path covers both flat text and runaround. Reflow-on-drag is bounded by the viewport,
not the document.

Caution — **BiDi × runaround is a feature-multiplication zone.** Flowing RTL text
around an exclusion reverses slot order within a row; mature DTP tools have shipped
bugs here. Land the slot model and BiDi *separately*, prove each, and only then let
them interact.

---

## 4. Virtualization: two machines, one viewport

These are *different* machines and must not be conflated. They share only a viewport
abstraction.

### 4.1 Row virtualization (a million discrete records)

Unlocks: LDAP user/group lists, SQL table browsers, any large record set.

A windowed provider that never materializes the full set. The critical fact: the
backends do **not** offer cheap random access. `OFFSET 750000` is O(n) in Postgres,
and LDAP has no native offset at all. Therefore the provider trait is
**seek/keyset-shaped**, not offset-shaped:

```
trait RecordSource {
    type Key;
    type Row;
    // Fetch up to n rows whose key is immediately after `key` (or from the start).
    fn fetch_after(&mut self, key: Option<Self::Key>, n: usize) -> Window<Self::Row>;
    // Fetch up to n rows whose key is immediately before `key`.
    fn fetch_before(&mut self, key: Option<Self::Key>, n: usize) -> Window<Self::Row>;
    // Approximate fractional position of a key in [0.0, 1.0], for the scrollbar.
    fn approx_position(&mut self, key: &Self::Key) -> Option<f32>;
    // Exact total, if and only if the source cheaply knows it.
    fn exact_len(&mut self) -> Option<u64>;
}
```

This maps cleanly onto SQL **keyset pagination** (`WHERE key > ? ORDER BY key LIMIT
n`) and onto LDAP's **VLV control** (designed precisely to answer "give me 20 entries
around offset X%").

**Scrollbar honesty:** over a remote cursor source the thumb is an *estimate*, not a
true ordinal, unless `exact_len` returns `Some`. Render the approximation
deliberately rather than faking precision. (Contrast with graph scrollbars in 6.7,
which are exact because the canvas bounding box is known.)

### 4.2 Wrapped-line virtualization (one enormous flowed document)

Unlocks: large log/text viewers; reading a huge attribute value or document body.

Harder than row virtualization because **line count depends on width** — you cannot
jump to "wrapped line 750,000" without knowing where it falls. Solution: a lazy
`byte-offset → line` index built incrementally as the user scrolls or seeks, cached,
and invalidated on width change. Keep this entirely separate from row virtualization;
they share the viewport abstraction and nothing else.

---

## 5. Node graphs

Unlocks: database design, node-graph editors, the soft-synth, network/infra
overviews, the assembly-snippet IDE.

### 5.1 Sockets as semantic edge gaps

A socket is a `BorderGap` **with semantics**: a `(side, offset, direction, type)`
tuple anchored to a connector, rather than a decorative opening. That sockets fall
out of an existing primitive is the signal the feature belongs *in* mullion. The
gap-interval geometry is already proven (see Section 7, the surf field).

### 5.2 Orthogonal connector routing

This is the hard core, and it is a known-hard problem with a name: **orthogonal
connector routing**. References: libavoid (Adaptagrams; Inkscape/Dia) and ELK's
routers. Terminal scale rescues us — dozens of connectors over roughly an 80×200
grid, not a 10k-net PCB — so the approach is:

- **Grid A\*** over free cells (from the free-space structure), with a **heavy bend
  penalty** so the search prefers long straight channels and few corners. This yields
  the "train tracks / tin lines on a board" look.
- Route in **canvas space**, not viewport space (see 6.x). Routes are then stable
  under scrolling and are recomputed on graph *edits*, not on camera motion.

**Crossing ambiguity (honest limitation):** single-line box drawing has no
"hop-over" glyph, so a crossing that is not a join reads ambiguously. Mitigate with
**color-per-net** or with routing that avoids crossings — not with a jump glyph,
which the charset cannot express cleanly.

### 5.3 Nudging

When parallel connectors share a gutter, spread them onto **separate integer
tracks**. This imposes a **gutter capacity constraint** (a 2-cell gutter holds 2
parallels, no more), which the router must respect. The existing junction logic
(box-drawing glyph resolution where borders meet) extends to connector crossings and
T-joins.

### 5.4 Placement: manual + automatic

- **Manual:** floating sub-tiles (the nodes) are placeable by **mouse drag** and by
  **keyboard** (directional nudge / grid snap), reusing the floating-tile foundation.
- **Automatic:** **layered (Sugiyama) layout** — assign layers along the dataflow
  direction, order within layers by **median/barycenter** to cut crossings, snap to
  the grid. For non-DAGs, break cycles first with a **feedback-arc-set** heuristic.
  This is the dagre / Graphviz-`dot` / ELK-layered family and the right default for
  port-directed graphs.

### 5.5 Nesting and taps (the deep tail — schedule last)

- **Nesting:** a sub-tile that is itself a graph *and* a node in its parent's graph,
  with inner nodes connected to the parent's own sockets (inputs/outputs of the
  group). This is **hierarchical layout with port constraints** — ELK is one of the
  few systems that does it well, and it is not small.
- **Taps / fan-out:** one output feeding many inputs over a shared trunk is **not**
  point-to-point routing; it is a **rectilinear Steiner tree**, a distinct and harder
  optimization.

Neither is a reason not to do it; both are reasons not to do it in v1.

### 5.6 Semantic (level-of-detail) zoom

Terminal cells do not scale continuously — there is no 1.7× cell — so "zoom part-way
to reveal structure" cannot mean optical magnification. It means **level-of-detail**:
as a tile is allocated more cells, it crosses thresholds that swap its rendering for a
denser one:

  collapsed node → titled node → node with visible ports → node with full internal graph

Two cooperating mechanisms:

- **Continuous area animation** — the tile *growing* — driven through the layout
  solver by animating its constraints. This technique is already demonstrated in the
  `spiral_stress` example (its animated zoom grows a tile smoothly via the solver
  rather than the discrete `Tree::zoom_to` jump).
- **Discrete LoD thresholds** keyed on available area, swapping the renderer.

The discreteness of cells and the discreteness of detail levels line up, making this
a *better* fit for terminals than smooth optical zoom would be. A zoom focus target
may be a tiling child, a floating child, or a node inside a nested graph.

### 5.7 Graph viewport: 2D pan-and-cull

Unlocks: navigating large network/infra graphs and big schemas.

The graph lives on a logical **canvas** larger than its tile; the tile is a window;
pan is a `(dx, dy)` offset moved by keyboard (arrows / `hjkl`) and mouse
(drag / wheel), in all four directions.

- **Virtualization here is plain culling** — draw only nodes and connectors
  intersecting the visible window plus a margin. Graphs have dozens of nodes, not a
  million, so the heavy paging machinery stays on the row/text side.
- **Scrollbars are exact** here, because the canvas bounding box is known — a clean
  contrast with the estimated scrollbar over a remote record cursor. Same widget, two
  truth-levels.
- **Canvas-space routing makes scrolling calm:** because routes are computed in canvas
  coordinates and the canvas is unchanged by panning, the tracks stay put as you
  scroll instead of crawling. Re-routing is triggered by edits, not by camera motion.

---

## 6. Cross-cutting decisions (locked)

These were settled during design and apply throughout:

1. **Reroute every net per frame.** At terminal scale this is cheap and far simpler
   than incremental routing. (Combined with canvas-space routing, "every frame" still
   produces identical stable routes until an edit changes the canvas.)
2. **Scrollbar thumb may be an honest estimate** over remote cursor sources; it is
   exact only when the source cheaply knows its length, and exact for graph canvases.
3. **BiDi runs through everything that flows** — prose, table cells, and node labels
   — so there is no bidi-correct/bidi-incorrect seam to discover later.
4. **Additive-only** until a deliberate, coordinated version bump: `aerie` at 1.0.3
   shares the checkout and must keep compiling.
5. **Canvas-space routing** for all connectors (see 5.2, 5.7).

---

## 7. The surf field: parts donor, not a feature

The `spiral_stress` example contains what the author calls the "surf field": its
`side_gaps` generator produces animated gap intervals along each tile edge that
drift, pulse in width, and split/merge, with `stream_color` filling each gap with a
hue that scrolls along the edge.

It is **not** a feature to keep, but it de-risks the node-graph work and donates two
kernels:

- **Gap-interval geometry** — computing edge gaps, clamping to valid edge-local
  indices (`make_gap`), offsetting edge-local→absolute, robust at every box size. This
  is exactly the **socket primitive**. The hardest-looking part of sockets (placing
  and sizing edge gaps correctly at all scales) is already proven to work.
- **Streaming gradient along a gap** — directly reusable as **connector-flow
  animation**: scrolling a gradient down a wire to show direction or activity (signal
  flow in the soft-synth, data flow in the IDE). The technique is demonstrated.

Drop the **autonomous wandering** (real sockets are pinned to connectors and do not
drift) and refactor the two kernels into mullion proper; they are currently drawn
through the demo's private `Painter`, not a public API.

---

## 8. Build order

Each step builds on the previous. Effort balloons nonlinearly in exactly two places —
**BiDi × runaround** and **nesting × taps × routing** — so both are deliberately late
and isolated.

1. **Floating tiles + free-space/slot model** (the shared foundation) — and define the
   seek-shaped `RecordSource` trait alongside it.
2. **Text engine core** — grapheme/width, line-break, BiDi pipeline, logical↔visual
   cursor map, pagination, scrolling. (LTR milestone first; bidi machinery present.)
3. **Row virtualization** over the seek provider — million-record scroll, estimated
   scrollbar.
4. **Wrapped-line virtualization** — lazy byte→line index for huge flowed documents.
5. **Runaround** — slot-stream flow around floating tiles; then carefully, BiDi ×
   runaround.
6. **Sockets / ports** — lift gap geometry from the surf field; `BorderGap` with
   semantics; connector-flow gradient.
7. **Manual node placement** — mouse + keyboard placement of floating sub-tiles on a
   graph canvas.
8. **Orthogonal routing** — grid A\* over free cells with bend penalty, in canvas
   space.
9. **Nudging** — parallel connectors onto separate tracks; gutter capacity; crossing /
   T-join glyph resolution.
10. **Graph viewport** — 2D pan-and-cull, exact scrollbars, keyboard + mouse.
11. **Semantic / LoD zoom** — area thresholds reveal structure; integrate with
    solver-driven animated zoom; focus targets.
12. **Sugiyama auto-layout** — layering, barycenter crossing reduction, cycle
    breaking.
13. **Nesting + taps** (deep tail, optional v2) — hierarchical layout with port
    constraints; rectilinear Steiner trees for fan-out.

---

## 9. Non-goals and boundaries

- No optical (sub-cell) zoom; LoD only.
- No hop-over crossing glyph; crossings disambiguated by color or avoidance.
- No general constraint-based diagram layout in v1 beyond layered (Sugiyama); force-
  directed and orthogonal-compaction layouts are out of scope for now.
- Random-access record providers are not assumed; the seek/keyset shape is mandatory.
- mullion remains content-agnostic: domain widgets (LDAP attribute editors, ER
  entities, synth modules, assembly blocks) live in the *consuming* applications, not
  in mullion. mullion provides tiles, free space, text, sockets, routing, and
  navigation — never domain semantics.

---

## 10. Testing posture

Honor the existing culture: `TestBackend` for headless rendering assertions and
`proptest` with a checked-in regression corpus. New invariants that especially want
property tests:

- Free-space intervals never overlap floating children and always lie within the
  parent.
- Slot flow places no glyph outside its slot; total flowed width ≤ available width.
- Logical↔visual cursor map is a bijection per visual line; round-trips are identity.
- Router never places a connector cell on an occupied (node or border) cell.
- Nudging never exceeds gutter capacity.
- Keyset windows are contiguous and gap-free when stitched (`fetch_after` then
  `fetch_before` across the same boundary agree).
