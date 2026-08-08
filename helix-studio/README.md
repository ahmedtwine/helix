# helix-studio

The IDE layer for this Helix fork. It owns the `hx` binary, ships the default
config, and replaces Helix UI with its own.

Helix has no plugin system. The Steel plugin PR (#8675) is a conflicting draft
and its scripts cannot touch render code, so this crate hooks into `helix-term`
directly through a small inversion-of-control slot.

**Keep this file current.** Every new hook point and every customization goes in
the tables below.

## Why the binary lives here

`Component`, `Compositor`, and `Context` are defined in `helix-term`. This crate
uses them, so it must depend on `helix-term`. Rust forbids the reverse edge, so
`helix-term` can never call into this crate by name.

```
        FORBIDDEN                          ACTUAL

   helix-studio                      helix-studio  ──depends on──▶  helix-term
        │  ▲                              │                             ▲
   depends │ depends                  installs                          │
        ▼  │                          hooks at                     reads hooks
   helix-term                          startup                      at runtime
                                           │                             │
   cycle: will not compile                 └────▶  ui_hooks::HOOKS  ◀────┘
                                                   (OnceLock slot)
```

`helix-studio/src/main.rs` calls `install()` before `helix_term::entry::run()`.
That is the only moment the concrete type is known to both sides.

`helix-term`'s own binary is renamed `hx-vanilla`. Build stock Helix to compare
against with `cargo build -p helix-term`.

## Where the hooks fire

```
  hx (helix-studio/src/main.rs)
    │
    ├── helix_studio::install()  ──▶ fills the OnceLock
    │
    └── helix_term::entry::run(helix_studio::config::load)
          │
          ├── config::load()
          │     ├── config/default.toml         shipped defaults
          │     ├── ~/.config/helix/config.toml user, merged over defaults
          │     └── .helix/config.toml          workspace, if trusted
          │
          └── Application::new
                ├── handlers::setup() ──▶ events::register()
                │
                ├── claims_startup(cx) ─── will studio handle startup?
                │     └── if yes, Helix skips its directory picker
                │
                ├── open(StartupDirectory) ── only when nothing claimed it
                │
                ├── editor.new_file(..)   ◀── FIRST VIEW EXISTS ONLY AFTER HERE
                │
                └── startup(editor, cx) ─── panel::install(), session restore,
                                            register_hook! on the event streams
                                            MUST run after the first view
```

```
                       ┌──────────────────────────────────────┐
   a command wants     │  Compositor                          │
   to show some UI     │                                      │
         │             │   layers: Vec<Box<dyn Component>>    │
         │             │     ┌──────────────────────────┐     │
         ▼             │     │ EditorView               │     │
   push_layer(c) ──────┼───▶ │ Overlay<Picker>          │     │
         │             │     │ Popup                    │     │
     ┌───┴────┐        │     └──────────────────────────┘     │
     │ mount  │◀───────┼── Compositor::push                   │
     └────────┘        │                                      │
   wrap / resize /     │   render(area, surface, cx)          │
   replace anything    │     for layer in layers { .. }       │
                       │            │                         │
                       │       ┌────┴──────┐                  │
                       │       │render_top │  draw over all   │
                       │       └────┬──────┘  layers          │
                       └────────────┼─────────────────────────┘
                                    │
                              panel::render  ── every Panel paints here

   open(UiRequest) ── asked before helix-term builds its own component
```

## The panel system

Every studio surface is the **same component type**. A `Panel` is geometry plus
a content `Source`; the config decides where it sits and whether it exists at
all. This is modelled on Helix's own `Info` box (`helix-view/src/info.rs`), which
is not a compositor layer at all — it is data that `EditorView::render` paints
over the frame. Because it never becomes a layer, it never steals focus.

```
   config/studio.toml
        │
        │  [panel.tips]              [panel.sidebar]
        │  side   = "bottom"         side   = "left"
        │  width  = "fill"           width  = "25%"
        │  height = "1"              height = "fill"
        │
        ▼
   Placement { anchor, side, align, width, height, offset }
        │
        │  resolve(viewport, anchor_rect, content) -> Rect
        ▼
   ┌──────────────────────────────────────────────┐
   │  Panel   (the one component type)            │
   │                                              │
   │   clear_with(style) → Block → padding        │
   │                    │                         │
   │                    ▼                         │
   │            Box<dyn Source>                   │
   └──────────────────────────────────────────────┘
        ▲               ▲                ▲
        │               │                │
   recommender/     sidebar/          diff/
     engine.rs        modes.rs          engine.rs  hunk pairing
     mod.rs           mod.rs            mod.rs     two-column view
     scoring          explorer + git    side by side
```

Adding a fourth panel costs a `Source` impl, one arm in `panel::build`, and a
config block. It costs **nothing** upstream.

Both ends of a panel are driven by streams that studio already owns:

```
   PostCommand ────────┐
   OnModeSwitch        │
   DocumentDidOpen ────┼──▶ panel::observe(StudioEvent, &mut Editor)
   tab hover ──────────┘        passive tap, no Context, cannot consume

   ui_hooks::input ───────▶ panel::handle_input(&Event, &mut Context) -> bool
                                full Context, may consume the event
```

`observe` is the passive stream: it fires from `register_hook!` callbacks where
only `&mut Editor` exists. `handle_input` is the interactive path and carries a
full `&mut Context`, so a `Source` can run a whole `Component` inside its own
resolved `Rect`.

### Signal vocabulary

Gates in `config/commands.toml`. A group or entry shows only when **every** token
in its `when` list holds. Prefix any token with `!` to negate it.

| Token | Holds when |
| --- | --- |
| `normal` `insert` `select` | The editor is in that mode |
| `pending:goto` `pending:space` `pending:match` `pending:window` `pending:view` | That prefix menu is open |
| `selection` | The primary selection covers more than one character |
| `linewise` | The selection starts at a line start and ends at a line break |
| `multiline` | The selection spans more than one line |
| `multi-cursor` | More than one selection range |
| `modified` | The buffer has unsaved changes |
| `lsp` | A language server is attached |
| `diagnostics` | The document has diagnostics |
| `buffers` `splits` | More than one document, more than one view |
| `focus:tab` `focus:picker` `focus:editor` | Where the mouse or UI focus is |

Ranking is `weight * 4`, plus `GROUP_BONUS` when a group's `after` list matches a
recent command, plus `EDGE_BONUS` for an entry's own `after`, both decayed by how
far back the match was, plus a learned bigram count, minus `REPEAT_PENALTY` for
the command that just ran. `GROUP_BONUS` equals `EDGE_BONUS` deliberately: the
group you just engaged with must be able to outrank a group that is merely gated
on ambient state, or searching would bury `n` under the selection groups.

**Every entry carries an explicit `weight`**, and a test enforces it. Without one
an entry inherits the group base, every entry in the group ties, and the sort
falls back to file order — which is how `x line` ended up last in the `Select`
group instead of second.

**`repeat = true`** exempts an entry from `REPEAT_PENALTY`. Some commands are
meant to be pressed again — `x` adds another line, `n` finds the next match, `w`
walks another word. Demoting those after one press is exactly backwards. The
penalty still applies to one-shot commands like `i` or `␣y`.

An entry's `after` list must stay narrow. `mif` once listed `extend_line_below`,
so pressing `x` gave it a full `EDGE_BONUS` and it leapfrogged both `word` and
`line`. Nesting edges should point from a selection to a *wider* selection of the
same kind, not across unrelated groups.

### Placement vocabulary

| Field | Values | Meaning |
| --- | --- | --- |
| `anchor` | `viewport` `editor` `cursor` `status` | The rect the panel is placed against |
| `side` | `top` `bottom` `left` `right` `over` | Which edge of the anchor it hugs, inside it |
| `align` | `start` `center` `end` | Position along the cross axis |
| `width` / `height` | `fit` `fill` `40` `25%` | `fit` asks the `Source` for its natural size |
| `offset` | `[dx, dy]` | Applied last, then clamped into the viewport |
| `border` | `true` `false` | Draws a `Block`, shrinking the inner rect |
| `padding` | `[h, v]` | Inset applied after the border |
| `style` | theme key | Background, e.g. `ui.statusline` |
| `order` | integer | Paint order; higher paints later |
| `toggle` | key, e.g. `"C-e"` | Shows and hides the panel. A panel with a toggle is built even when `enabled = false` |

`Fit` sizes against the **viewport**, `Fill` and percentages against the
**anchor**. That distinction matters: `anchor = "cursor"` is a 1×1 rect, so
sizing `Fit` against the anchor collapsed the hover popup to a single cell.

`anchor = "editor"` is the viewport minus the bufferline and the commandline, so
a panel anchored there never covers the tab strip or the prompt.

### Why panels overlay rather than reserve

A panel paints over the frame; it does not shrink the editor. Reserving a row
would mean editing `EditorView::render`, where `area.clip_bottom(1)` is hard
coded for the commandline. More importantly the statusline is drawn at the
bottom of each **view** (`ui/editor.rs`), so with splits there is no single
statusline to sit above. The default `tips` placement therefore lands on the
commandline row, which is blank unless a status message or a pending key is
showing.

## Hook points

All defined in `helix-term/src/ui_hooks.rs`. Every one defaults to a no-op, so
stock Helix behaviour is unchanged when nothing is installed.

| Hook | Called from | Reach |
| --- | --- | --- |
| `claims_startup(cx) -> bool` | `application.rs`, before file loading | Tell Helix to skip its startup picker |
| `startup(editor, cx)` | `application.rs`, after the first view exists | Restore session, install panels, register event hooks |
| `mount(component)` | `compositor.rs`, `Compositor::push` | Wrap, resize, or replace any component before it becomes a layer |
| `render_top(area, surface, cx)` | `compositor.rs`, `Compositor::render` | Draw over every layer, every frame. Drives the whole panel system |
| `render_bufferline(editor, area, surface) -> bool` | `ui/editor.rs`, `EditorView::render` | Own the tab strip; return `false` to fall back to Helix's |
| `input(event, cx) -> bool` | `ui/editor.rs`, `EditorView::handle_event` | First refusal on every key and mouse event, with a full `Context` |
| `open(request, editor)` | `commands.rs`, `application.rs` | Build a component instead of Helix's |

`UiRequest` variants: `FilePicker`, `FileExplorer`, `StartupDirectory`.

Returning `None` from `open` falls back to Helix's component, so surfaces can be
taken over one at a time.

### Event streams, which cost no hooks at all

`PostCommand` and `OnModeSwitch` are already declared and registered by Helix in
`helix-term/src/events.rs`, and `PostCommand` is dispatched in
`ui/editor.rs` right after `command.execute(cxt)`. Studio subscribes with
`register_hook!` exactly like `handlers/completion.rs` does. The whole command
stream that feeds the recommender needs **zero** upstream changes.

### Hook design rule

**helix-term supplies facts, helix-studio decides policy.** A hook that takes no
arguments forces the decision upstream, which means editing Helix every time the
rule changes. `StartupContext` carries `root`, `opening_directory`, `file_count`,
and `tutor` so studio alone decides whether to restore a session.

### Ordering constraints

Rules that are not obvious and will crash or silently no-op if broken:

- `startup` must run **after** `editor.new_file(..)`. `Editor::open` calls
  `switch`, which indexes `tree.focus`; before the first view exists that
  unwraps `None` and panics at `helix-view/src/tree.rs:301`.
- `register_hook!` must run **after** `events::register()` (`handlers.rs:31`,
  reached from `application.rs`). That is why hooks are registered inside
  `startup` rather than `install()`.
- The panel registry is a `thread_local!`, not a `static Mutex`. `install`,
  `render`, `handle_input`, and `observe` all run on the main thread, and a
  `Source` may hold non-`Send` state such as a `Picker`.

## Customizations

| Area | Change | Where |
| --- | --- | --- |
| Config | Ships `config/default.toml` as built-in defaults, merged under user config | `config.rs` |
| Panels | One component type, config-driven geometry, pluggable content | `panel/` |
| Tips footer | Predicts the next commands from state and history | `panel/recommender/` |
| Sidebar | 25% drawer with explorer and git-changes modes | `panel/sidebar/` |
| Diff | Side-by-side hunks against the VCS base, click to stage | `panel/diff/` |
| Hover | Rest on a symbol and the LSP signature plus cursor diagnostics float above it | `panel/hover/` |
| Keymap | `space f` finds files, `space e` opens the explorer, rest trimmed | `config/default.toml` |
| File picker | Filename-first column plus a dimmed directory column | `picker.rs` |
| File explorer | Recursive tree, always opens at the workspace root, reveals the current buffer | `explorer.rs`, `tree.rs` |
| Overlay | Centered 80%×70% instead of Helix's near-fullscreen `overlaid` | `overlay.rs` |
| Session | Remembers open files per workspace, restores on launch | `session.rs` |
| Icons | Disclosure triangles; Nerd Font glyphs behind `NERD_FONT` | `icons.rs` |
| Tabs | Click to switch, hover reveals a close `×` in place | `chrome.rs` |
| Mouse | Scroll and click-to-select in every picker | upstream `ui/picker.rs` |

### Notes on specific customizations

**Recommender.** The engine is pure: `Signals` in, ranked `Section`s out, no
rendering and no editor types beyond `Mode`. Groups and entries live in
`config/commands.toml` and are gated by tokens (`normal`, `lsp`, `selection`,
`focus:tab`, …) that must **all** hold. Ranking is `base * 4`, plus a recency
bonus when a listed `after` command appears in the last eight commands, decayed
by distance, plus a learned bigram count, minus a penalty for the command that
just ran. The bigram table makes it adapt within a session and is the hook for
longer-horizon memory later.

A test asserts the catalog never teaches a `space` key that `default.toml`
unbinds, so trimming the keymap can never leave the panel lying.

**Diff.** No new dependency was needed. `imara-diff` is already a direct
dependency of both `helix-vcs` and `helix-core`, and `Document::diff_handle()`
already carries the computed hunks that draw the gutter markers. The engine only
*pairs* them: for a hunk it emits `max(removed, added)` rows, padding the shorter
side with `None`, so a pure insertion has an empty left column. Runs of unchanged
lines beyond the context window collapse to a single gap row, and a gap is never
emitted before the first hunk.

Staging shells out to `git add`. `gix` is present but its index-write API is the
least settled part of the crate, and `git2` would pull in a second git stack
next to the one Helix already links. Shelling out is what lazygit does.

**helix-tui is a fork of tui-rs**, ratatui's ancestor. The widget model is the
same (`Widget`, `Buffer`, `Block`, `Paragraph`, `Table`, `List`) but the crate
identity differs, so ratatui-ecosystem widgets cannot be depended on, only
ported.

**Auto-hover is the one LSP thing Helix does not have.** `signature_help`
(`handlers/signature_help.rs`) fires in *insert* mode inside a call, and `hover`
(`commands/lsp.rs`) is manual only — there is no `handlers/hover.rs`. Studio adds
idle hover in normal mode using Helix's own parts: `AsyncHook` for the debounce,
`SelectionDidChange` for the trigger, `job::dispatch` to get the reply back onto
the main thread. `handlers/document_highlight.rs` is the template.

The popup also folds in diagnostics at the cursor, so an error under the symbol
shows in the same box rather than only in the gutter.

**Staying in sync with Helix's own menus.** Pressing `g`, `space`, `m`, `C-w`, or
`z` puts Helix into a pending keymap node and sets `editor.autoinfo` to that
node's `infobox()`, whose title is the node name — `"Goto"`, `"Space"`,
`"Match"`, `"Window"`, `"View"` (`helix-term/src/keymap.rs`, node names in
`keymap/default.rs`). `EditorView::render` takes that `Info` and puts it back, so
by the time `render_top` runs it is readable. The recommender turns it into the
`pending` signal, and `suggest` then shows **only** groups gated on a matching
`pending:` token, hiding the normal ones. An unrecognised popup falls back to the
normal groups instead of going blank.

This costs no hook and no event. The panel re-reads `autoinfo` every frame, which
is exactly when it can change.

**Selection is a state machine, not a flag.** `linewise` and `multiline` are
derived from the primary range against the rope, so the panel offers different
moves depending on the shape you are in: `X` snap-to-line only when the selection
is *not* already linewise, `A-i` shrink and `s` split-into-cursors only when it
spans lines, `C` cursor-below only when it is linewise. Gates support `!` for
negation, which is what makes those complements expressible in config.

**Grid layout.** `rows` allots a maximum height and `pack` fills it greedily.
Keys and their labels are one unbreakable unit, so a wrap never orphans a label
from its binding. With `grid = true` every key/label pair is padded to a common
width, so items line up in columns across rows. `height = "fit"` then asks the
source how many rows it actually needs, and the panel shrinks back to one row
when the content fits.

**Tab hover feeds the recommender.** `chrome` reports its hovered tab through
`StudioEvent::Hover`, which flips the `focus:tab` gate and makes the panel show
buffer and split bindings for the tab under the mouse.

**Explorer.** Helix descends into a directory by pushing another picker layer, so
five levels deep means five live pickers. Studio removes and replaces the single
layer, and the tree keeps its expanded state, so parent and child are visible
together. It always opens at the working directory and calls `Tree::reveal` to
expand the path down to the current buffer.

**Tree.** `rows` is a flat `Vec` where each row carries its `depth`. `expand` is
`Vec::splice` at `index+1`, `collapse` scans forward to the first row with
`depth <= parent.depth` and does one `drain`. `reveal` walks the path components
and expands each ancestor in turn. No recursion, no parent pointers.

**Session.** Written to `~/.cache/helix/studio-session.toml`, keyed by workspace
root, capped at `MAX_FILES`. Saved on `DocumentDidOpen` and `DocumentDidClose`
rather than at exit, so it survives a crash or a `kill`.

**Overlay width.** `drawer_left` (25%) exists in `overlay.rs` but is unused: a
quarter-width panel falls under `MIN_AREA_WIDTH_FOR_PREVIEW` (72 columns) and
would suppress the preview pane.

**Tabs.** Each tab reserves a trailing cell that stays blank until hovered, so
revealing the `×` never reflows the strip. The `×` is painted with a fg-only
`Style`: `Cell::set_style` (`helix-tui/src/buffer.rs:88`) only overwrites `bg`
when the style sets it, so the tab keeps its own background and the theme's
`diagnostic.error` underline is not inherited.

`EditorView` cannot be wrapped via `mount` to achieve this.
`compositor.find::<ui::EditorView>().unwrap()` appears in six places, including
`commands.rs` and `handlers/completion.rs`; a wrapper makes `find` return `None`
and those unwraps panic. Hence the two dedicated hooks.

## Modules

| Module | Role |
| --- | --- |
| `lib.rs` | `Studio`, the `UiHooks` impl, `install()`, event-stream registration |
| `config.rs` | Merges `config/default.toml` under the user config |
| `panel/mod.rs` | `Panel`, the `Source` trait, `StudioEvent`, the registry, the config schema |
| `panel/layout.rs` | `Placement` geometry: anchor, side, align, extent, offset |
| `panel/recommender/engine.rs` | Catalog, signals, scoring, history, learned edges |
| `panel/recommender/mod.rs` | Tips view: reads editor state, paints the row |
| `panel/sidebar/modes.rs` | Explorer and git modes, background changed-file scan |
| `panel/sidebar/mod.rs` | Drawer view: rows, scrolling, mouse |
| `panel/diff/engine.rs` | Pairs hunks into aligned left/right rows; stage and unstage |
| `panel/diff/mod.rs` | Two-column view with gutters, markers, and gap elision |
| `panel/hover/engine.rs` | Debounced idle detection, the LSP hover request, content modes |
| `panel/hover/mod.rs` | Popup view: signature or documentation, plus cursor diagnostics |
| `chrome.rs` | Tab strip rendering, hover tracking, click-to-switch and close |
| `explorer.rs` | Tree-backed file explorer built on Helix's `Picker` |
| `tree.rs` | Flattened-vector directory tree with expand, collapse, and reveal |
| `session.rs` | Per-workspace open-file persistence |
| `overlay.rs` | Centered and left-drawer overlay sizing |
| `picker.rs` | Fuzzy file picker with filename-first columns |
| `icons.rs` | Glyph table by file extension |

## Config precedence

Editor config, lowest to highest:

1. `helix-studio/config/default.toml` — shipped with the binary
2. `~/.config/helix/config.toml` — user
3. `.helix/config.toml` — workspace, only when trusted

Panel config, lowest to highest:

1. `helix-studio/config/studio.toml` — shipped with the binary
2. `~/.config/helix/studio.toml` — user

Both merge with `helix_loader::merge_toml_values` at depth 3, so setting one key
does not drop the rest of a table.

The command catalog is `helix-studio/config/commands.toml`. All three are
`include_str!`-ed into the binary, so changing them needs a rebuild **and** a
restart; `:config-reload` will not pick them up.

## Upstream diff

Changes to files Helix owns, kept small so rebases stay cheap:

| File | Change |
| --- | --- |
| `ui_hooks.rs` | New. The hook trait and its `OnceLock` slot |
| `entry.rs` | New. `main.rs` body moved here so both binaries share it |
| `compositor.rs` | 2 lines, calls `mount` and `render_top` |
| `application.rs` | `claims_startup`, `startup`, and the `StartupDirectory` request |
| `commands.rs` | Picker commands route through two helpers |
| `ui/editor.rs` | 2 call sites: `render_bufferline` and `input` |
| `ui/picker.rs` | Mouse scroll and click-to-select |
| `ui/mod.rs` | `get_excluded_types` made public |
| `lib.rs` | `filter_picker_entry` made public, two module declarations |
| `main.rs` | Reduced to a thin wrapper |

Two of those files are new and can never conflict. Everything else in this crate
is unreachable from upstream. Note that `render_top` alone carries the entire
panel system, so new panels cost zero upstream changes.

## Tests

`cargo test -p helix-studio` — 80 tests:

- config merge layers, for both the editor config and the panel config
- tree splice, drain, and reveal invariants, including the exact depth sequence
  after a nested expand, which row counts alone would not catch
- placement geometry: every extent spelling, offset clamping, cross-axis align
- recommender ranking: gating, mode swaps, recency decay, learned edges, the
  repeat penalty, and a broken catalog degrading to empty instead of panicking
- pending-prefix takeover, per-prefix mapping, and the unrecognised-popup fallback
- the selection state machine, including negated gates
- row packing: wrapping, the row budget, and keys never orphaned from labels
- diff hunk pairing: insertions, deletions, uneven hunks, gap elision
- the catalog-versus-keymap consistency guard

Not covered by unit tests, because they need a live terminal: panel painting,
prefix-menu sync, session round-trip, and the explorer's expand/collapse rebuild.
Those run under a pty harness of 19 scenarios that resizes the terminal to force
a full repaint, then greps the reconstructed screen. Without the resize,
`helix-tui` only rewrites changed cells, so strings split across frames and the
grep produces false misses.
