# rbx_dom_weak 4.2.0, patched for rustudio

Vendored from crates.io (`rbx_dom_weak` 4.2.0, part of
[rbx-dom](https://github.com/rojo-rbx/rbx-dom), MIT; `LICENSE.txt` is
upstream's licence). Used through `[patch.crates-io]` in the root
`Cargo.toml`.

Changes, all in `src/dom.rs` and marked `rustudio:`:

- `WeakDom::detach_children(referent)`: detaches every child of an
  instance at once (each child's parent becomes none) and returns them in
  order, in O(children).
- `WeakDom::detach_descendants(referent)`: detaches a whole subtree,
  returning every descendant (pre-order) with its former parent.
- `remove_child`: the places that took one child out of its parent's child
  list (`retain` over the whole list) now search from the end and remove
  just that entry.

Why: `Instance:Destroy()` and `ClearAllChildren()` on large models
(tens of thousands of parts) removed children one at a time, scanning the
parent's child list for each, which is quadratic. The new calls let
rustudio detach a subtree in one pass, and newest-first removal (what
ClearAllChildren does, as in Studio) no longer shifts the whole list.
Nothing else in the crate changed.
