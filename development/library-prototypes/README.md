# Historical library prototypes

These files preserve unfinished library drafts outside the compiled sysroot.
They are not executable APIs, tests, or language specifications. Keeping them
under library/ previously made every sysroot invocation fail name resolution,
even when the user only needed builtin iteration or arithmetic.

| Draft | Missing work before reintroduction |
| --- | --- |
| builtin/option_unwrap.fl, builtin/result_unwrap.fl | Define and implement panic/control-transfer behavior, including resource cleanup. Total optional/error-result operations remain in builtin. |
| builtin/formatting.fl | Design Display/Debug without depending on std.String. The old String-returning signatures are not the accepted final contract. |
| std/collections.fl | Real allocation/storage and resource contracts, language optional/error semantics, and source Iterator integration. The legacy intrinsic names and Some/None forms are not implementations. |
| std/fmt.fl | Owned String, comptime format descriptions and formatting protocol integration. |
| std/io.fl | Runtime output/input capabilities and actual platform bindings. |
| std/lib.fl | Historical exports only; restore exports as their implementations become usable. |

The accepted builtin-package plan's stage B explicitly separates unfinished
String/formatting dependencies so the real builtin package can load. This is
that boundary, not completion of stage E: allocation, containers, formatting,
I/O and owned strings remain required development work. Do not reintroduce
placeholder bodies or aliases merely to make the names resolve.
