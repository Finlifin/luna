# Conditional Copy and error wrappers

The conditional half-precision `Copy` implementation formerly made the
`impl Clone for !E T` body's `self.*` read fail during formation. This is fixed:
`native_resources.fl` now shares both Clone and Copy across f16/bf16.

The root cause was budget partitioning in world propositions. Direct trait
resolution selected the error-wrapper implementation and discharged its
payload capabilities. The surrounding proposition check divided its allowance
by the number of atoms, then reserved another fixed slice for selection inside
each decision. Unused work from cheap given/formation checks could not reach
the one expensive candidate search, which returned Pending.

Metered proposition decisions now share the caller's remaining allowance;
selection reuses work returned by formation checks. Input views also charge
actual traversal, copying, and equality-proof work instead of consuming an
entire reserved slot when an already-scoped type is returned unchanged. The total budget is not
increased. Evidence remains tied to the same implementation world and can be
replayed. Exhaustion does not publish assumptions or poison later queries.

Regression commands from the repository root:

```sh
python3 -B development/check.py --timeout 900 -- test --release -p type_resolution --test capabilities
python3 -B development/check.py --timeout 900 -- test --release -p luna --test library_protocols native_wrapper_copy_requires_every_payload_and_preserves_pointer_lifetime
python3 -B development/check.py --timeout 900 -- test --release -p luna --test frontend_cli sysroot_half_float
```

The solver regression combines many scalar candidates, a conditional generic
candidate, error-wrapper payload obligations, and several given trait atoms.
It checks bounded consumption, low-budget failure, subsequent recovery, and
world-scoped proof replay. Library regressions retain non-Copy payload and
pointer-lifetime rejection cases alongside VM/native half-precision execution.
