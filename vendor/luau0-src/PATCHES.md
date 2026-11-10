# luau0-src, rustudio patches

`luau/` is Luau (MIT, `LICENSE`). rustudio's changes are marked
`rustudio:` in the source.

## lua_hostcall (2026-11-09)

`luau/VM/include/lua.h`, `luau/VM/src/ldo.cpp`: a new
`lua_hostcall(L, delta)` adds `delta` (+1 to enter, −1 to leave) to the
thread's C-call depth.

Why: rustudio's fast paths for engine methods (`src/cmeta.rs`) run their
own code directly inside a C function that has a continuation. Luau
would count a Lua call made from there as yieldable, so a script `pcall`
inside it could yield and schedule a re-entry the host isn't ready for.
Marking that stretch as a host call makes Lua called from it never
yieldable, as from an ordinary C function.

## Earlier patches

Also marked `rustudio:`: `lua_setcfunctionname` (a C closure's debug
name), `lua_getinfoscript` (lua_getinfo as scripts see it: engine
`=rustudio.*` closures read as C functions), and `getfenv` never handing a
script the host's own globals table (`lbaselib.cpp`).
