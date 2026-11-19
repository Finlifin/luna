# Historical library prototypes

These files preserve unfinished library drafts outside the compiled sysroot.
They are not executable APIs, tests, or language specifications. Keeping them
under library/ previously made every sysroot invocation fail name resolution,
even when the user only needed builtin iteration or arithmetic.

| Draft | Missing work before reintroduction |
| --- | --- |
| builtin/option_unwrap.fl, builtin/result_unwrap.fl | Define and implement panic/control-transfer behavior, including resource cleanup. Total optional/error-result operations remain in builtin. |

The public `println` uses a comptime format string. At each call,
`std.fmt.Parse(format)` supplies the variadic domain and every field must
implement Display. The [native demo](../std-host-foundation.md) exercises
this interface through the real sysroot. The earlier formatting and I/O
stubs have been removed.

Allocation, containers, complete formatting, I/O and owned strings remain
required work in the [library delivery design](../libraries-and-delivery.md).
Do not reintroduce placeholder bodies or aliases merely to make names resolve.

`builtin/text_iteration.fl` is an executable algorithm prototype, not a
historical shipped interface. Its tests demonstrate byte order and loop control
in unchecked interpretation and retain the checked-execution rejection. It
requires [text-view provenance](../solvers-resources-and-lowering.md) before public export;
adding Copy alone cannot discharge that dependency.
