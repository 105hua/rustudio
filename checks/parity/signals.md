# Signal timing (Workspace.SignalBehavior)

Code: `src/runtime_signals.rs` (module `runtime::signal_timing`), hooks in
`src/runtime.rs` (`World::fire`, `Runtime::dispatch`, the method / index /
newindex / `__rs_fire` wrappers).

## Studio facts

- Enum.SignalBehavior: Default 0, Immediate 1, Deferred 2, AncestryDeferred 3.
- Studio's own docs (`content/api_docs/en-us.json`): Default is "currently
  equivalent to Immediate but this will eventually change to Deferred";
  AncestryDeferred = Deferred "only for events triggered by changes in
  ancestry".
- Workspace.SignalBehavior is NotScriptable, CanLoad false in api-dump; the
  place file stores it as `SignalBehavior2` (rbx-dom database). Both names
  are read; missing = Default = Immediate.
- Debug FFlags in Studio: `DebugForceDeferredSignalBehavior`,
  `DebugForceAncestryDeferredSignalBehavior` (not modelled).
- Deferred re-entrancy: FInt `MaxDeferReentrancyDepth` (name,
  value = 80). increments a byte counter, compares
  `> FInt`, logs at error level "Maximum event re-entrancy depth (%d)
  exceeded for %s.%s" and drops the invocation.
- Immediate re-entrancy message "Maximum event re-entrancy depth exceeded   for {}.{} when calling {} on line {} in {}" (, referenced at ).
- Also present: "Maximum re-entrancy depth (%i) exceeded calling task.defer"
.

## How rustudio does it

- `World::fire` can't run Lua (it holds the World borrow). Immediate fires
  go to `World::now`; `flush_immediate` runs them at the script API boundary
  after the engine call returned and dropped its borrows, before the script
  continues. It refuses to run while World / Signals / Sched / Cache /
  Reentry is borrowed up the stack (the 9981a2b double-borrow class); those
  fires wait for the next boundary or `dispatch`.
- Engine-originated fires (frame steps, input via `Runtime::fire`) run at the
  next `dispatch` (first thing each round), not inside `Runtime::fire`: play
  code may hold its own locks there.
- Deferred fires keep the old queue (`World::events`), now carrying a
  re-entrancy depth; handlers run at `dispatch` (after the script yields).
  Chains past `mech::MAX_DISPATCH_ROUNDS` (64) per frame continue next frame
  (Studio runs the whole chain at one resumption point).

## Left

- Exact Immediate depth limit (assumed 80).
- task.defer re-entrancy limit.
- Deferred: Studio flushes after each script resumption; rustudio flushes
  per dispatch round.
