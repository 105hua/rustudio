# mlua 0.12.2, patched for rustudio

Vendored from crates.io (`mlua` 0.12.2, MIT; `LICENSE` is upstream's
licence). Used through `[patch.crates-io]` in the root `Cargo.toml`.
Upstream's examples, tests and benches are left out of the copy.

Change, in `src/state.rs` and marked `rustudio:`:

- `Lua::with_current_state(state, f)` (unsafe, `#[doc(hidden)]`): runs `f`
  with the Lua's current thread set to `state`, then restores it, as mlua
  does around its own callbacks.

Why: rustudio's direct Instance bridge (`src/runtime_routes.rs`) answers
`__index` / `__newindex` / `__namecall` from raw C functions instead of
mlua callbacks. The Rust code it calls must see the Lua thread that made
the call (`current_thread`, `inspect_stack`, and any API call it makes),
which only mlua can set. Nothing else in the crate changed.
