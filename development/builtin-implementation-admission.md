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

The sysroot driver grants the semantic IDs of its actual six selected library
packages; it does not grant the application package or same-named source modules.
The package-set driver grants the explicitly designated `--builtin` package.
Embedding callers can supply additional selected library identities through
`Protocols.builtin_implementation_packages`; source selectors do not grant them.

Elaboration carries the same set into early and final trait/inherent indexes.
NIR retains the set, including required-invocation and resource snapshots, and
later VM/type/resource worlds reuse it when rebuilding indexes. VM world-table
construction charges the extra metadata. The meta.Object public API now uses inherent methods; its raw operations remain
in the centralized binding layer.

Verification: type_resolution integration suites builtin_ownership,
implementation, implementation_phases, and inherent. The new suite checks
unprivileged rejection, foreign nominal/normal-form rejection, primitive and
qualified-type admission, duplicate/budget failure atomicity, phase merging,
coherence, and deterministic dumps. The luna builtin_implementations suite
covers explicit driver grants, early Type methods, optional generic receivers,
all six sysroot providers, application rejection, duplicate inherent methods,
and a foreign-trait implementation through runtime instantiation. This verifies
the source admission path; the library Object facade is covered by library_meta_objects.

Native optional and error-qualified operations now live in `impl ?T where T`
and `impl !E T where T, E`. Their public methods no longer require OptionalOps
or ErrorResultOps imports or trait casts. This includes conditional callbacks,
nested optional layers, error mapping, and affine payload cleanup; it does not
add a library enum or change either native representation. Closure domains use
the centralized primitive Type binding.

The compiler-bound List family has a closed error row. PTC uses that fact to
exclude overlap with error-qualified inherent methods while retaining empty-row
normalization and opacity for unregistered normal-form heads. The real sysroot
fixture is `development/examples/sysroot-qualified-methods.fl`; isolated library
fixtures explicitly designate their provider package instead of granting
application packages an ownership exemption.
