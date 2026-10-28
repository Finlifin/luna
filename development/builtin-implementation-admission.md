# Builtin implementation admission

The author permits builtin, meta, build, attrs, std, and verification to add
inherent and trait implementations for compiler builtin types. Ordinary packages
do not receive that exemption, and foreign nominal types remain subject to the
usual ownership rules.

The implementation indexes now expose `for_builtin_packages(target, identities)`.
The supplied semantic package IDs are immutable for that index's lifetime;
`Default` and `for_target` retain ordinary ownership checks. The grant covers
primitive types and the pointer, optional, error-result, and callable type
constructors. It does not unfold normal-form heads or grant ownership of an
arbitrary type variable. Normal-form library constructors such as Tuple and List
still use their declared identities and existing ownership rules.

Trait admission, phase admission, and inherent admission use the same package
boundary. Duplicate declarations, coherence, budgets, and method selection retain
their existing checks. Dumps include the sorted authorized IDs.

Driver/elaboration wiring remains pending. It must derive the authorized IDs
from the selected builtin packages, retain them across early and final method
indexes, and distinguish real packages from same-named user modules. Do not
infer privileges from a meta.List selector or a user-written alias. Until that
wiring is complete, meta.Object's proposed inherent API still fails admission;
the working library retains its existing public functions.

Verification: type_resolution integration suites builtin_ownership,
implementation, implementation_phases, and inherent. The new suite checks
unprivileged rejection, foreign nominal/normal-form rejection, primitive and
qualified-type admission, duplicate/budget failure atomicity, phase merging,
coherence, and deterministic dumps. This is index-level verification, not proof
that the source-language Object method API works yet.
