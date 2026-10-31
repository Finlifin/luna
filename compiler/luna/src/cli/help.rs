//! Command-scoped help for the public CLI.
pub const USAGE: &str = "Usage: luna <command> [options]\n       luna help [command]";
pub const HELP: &str = "Luna — Flurry compiler\n\nUsage: luna <command> [options]\n\nCommands:\n  check       Type-check a source without emitting a dump\n  build       Build a host executable (or --emit object|bitcode)\n  test        Run tests (--engine native|reference; default native)\n  dump        Emit AST, HIR, NIR, IIR, LLVM, assembly or attributes\n  comptime    Invoke a comptime function\n  interpret   Execute a runtime entry in the reference interpreter\n  analyze     Inspect resources or text regions\n  packages    Apply a command to explicit package inputs\n  serve       Serve the versioned package stream protocol\n  help        Show command-specific help\n\nUse luna <command> --help for options and examples.\nSource-first invocations and --packages-v1/--packages-stream-v1 remain\ncompatibility entries; new automation should use subcommands.\n";
const FRONTEND: &str = "
Frontend options:
  --sysroot <path>
  --dispatch-fuel <steps>
  --build-mode <mode>
  --build-target <arch> <os> <bits>
  --build-feature <name=true|false>
  --package-build-mode <identity=mode>
  --package-build-target <identity> <arch> <os> <bits>
  --package-build-feature <identity> <name=true|false>
  --comptime-intrinsic <operation=selector> (repeatable)

Use luna help bindings for compiler protocol binding overrides.
Targets describe build context, not cross-compiling backend selection.
Paths need not be UTF-8. Use -- before a source path beginning with a dash.
";
const HOST: &str = "
Host options (build, native tests, LLVM/assembly dumps):
  --opt-level 0|1|2|3
  --host-intrinsic <operation=selector> (repeatable)
  --host-fma <selector>, --host-sqrt <selector>, --host-copysign <selector>
";
const PACKAGE_INPUTS: &str = "
Package inputs:
  --package <id> <identity> <name> <entry> (repeatable)
  --primary <id>
  --dependency <requester> <alias> <provider> (repeatable)
  --builtin <id>, --builtin-impl-package <id>
  --package-metadata-v1 <hex>
  --with-inputs (except check)
No positional source path is accepted.
";
const BINDINGS: &str = "Compiler binding overrides:
  --copy-protocol <trait>, --drop-protocol <trait>
  --splice-protocol, --meta-list-type, --tuple-type, --index-protocol,
  --ord-protocol, --eq-protocol, --list-pattern-protocol,
  --comptime-list-pattern-protocol,
  --literal-into-protocol, --iterator-protocol, --into-iterator-protocol,
  --add-protocol, --sub-protocol, --mul-protocol, --div-protocol,
  --rem-protocol, --neg-protocol, --not-protocol <source-selector>
Selectors use dotted source paths. Availability depends on the selected
pipeline. Normal applications obtain bindings from --sysroot.
";

fn command_help(name: &str) -> Result<String, String> {
    let usage = match name {
        "check" => {
            "check <input.fl> [options]\n  Check through typed NIR without emitting an output artifact."
        }
        "build" => {
            "build <input.fl> -o <path> [--entry <name>] [--emit executable|object|bitcode]\n  Entry defaults to main; output is required."
        }
        "test" => {
            "test <input.fl> [--engine native|reference] [--list] [--filter <text>]\n  --timeout-ms <ms> sets the native per-test deadline. --list only discovers\n  tests and cannot be combined with --engine or --timeout-ms."
        }
        "dump" => {
            "dump <input.fl> [--emit ast|hir|nir|iir|llvm|asm|attributes|merged-attributes]\n  Defaults to nir. --entry <name> (default main) applies to iir, llvm and asm."
        }
        "comptime" => {
            "comptime <input.fl> --entry <name> [--definition-fuel <steps>]\n  Invoke a comptime function and print its result."
        }
        "interpret" => {
            "interpret <input.fl> [--entry <name>]\n  Reference-execute a runtime entry (default main) and print its value."
        }
        "analyze" => {
            "analyze <resources|text-regions> <input.fl> [--entry <name>]\n  Entry defaults to main. resources requires --copy-protocol; optional\n  --drop-protocol supplies cleanup planning. Analysis is not verification."
        }
        "packages" => {
            "packages <check|build|test|dump|comptime|interpret|analyze> [command options]\n  --package <id> <identity> <name> <entry> (repeatable)\n  --primary <id> [--dependency <requester> <alias> <provider>]\n  [--builtin <id>] [--builtin-impl-package <id>] [--with-inputs]\n  [--package-metadata-v1 <hex>]\n  No positional source path; actions and options match source commands."
        }
        "serve" => {
            "serve packages-v1 [--cache-entries <count>]\n  Read versioned package requests from stdin; write protocol responses to\n  stdout. The request wire format remains unchanged. Default cache size: 8."
        }
        _ => return Err(format!("unknown command '{name}'")),
    };
    let mut text = format!("Usage: luna {usage}\n");
    if matches!(name, "serve" | "packages" | "analyze") {
        return Ok(text);
    }
    if name != "check" {
        text.push_str("\n  -o, --output <path>        Write output to a file\n");
        text.push_str("  --fuel <steps>             Action budget (dump: iir/llvm/asm only)\n");
    }
    if matches!(name, "build" | "test" | "dump") {
        text.push_str(HOST);
    }
    if matches!(name, "build" | "test" | "dump" | "interpret") {
        text.push_str(
            "\n  --inline-functions        Requires resource bindings; not test --list\n",
        );
    }
    if matches!(name, "build" | "dump") {
        text.push_str(
            "  --discard-result          Host output only; requires Copy/Drop bindings\n",
        );
    }
    text.push_str(FRONTEND);
    Ok(text)
}

/// Resolve a complete command path, shared by `help` and trailing `--help`.
pub fn help(path: &[&str]) -> Result<String, String> {
    match path {
        [] => Ok(HELP.into()),
        ["bindings"] => Ok(BINDINGS.into()),
        [name] => command_help(name),
        ["serve", "packages-v1"] => command_help("serve"),
        ["analyze", topic @ ("resources" | "text-regions")] => {
            let details = if *topic == "resources" {
                "Requires --copy-protocol <trait>; optional --drop-protocol <trait>."
            } else {
                "--inline-functions analyzes expanded bodies. Resource flags are not accepted."
            };
            Ok(format!(
                "Usage: luna analyze {topic} <input.fl> [options]\n  --entry <name> (default main)\n  --fuel <steps>\n  -o, --output <path>\n{details}\nAnalysis is not verification.\n{FRONTEND}"
            ))
        }
        ["packages", rest @ ..]
            if !rest.is_empty() && !matches!(rest[0], "packages" | "serve" | "bindings") =>
        {
            let text = help(rest)?;
            Ok(format!("{}{PACKAGE_INPUTS}", text.replacen("Usage: luna ", "Usage: luna packages ", 1)
                .replace(" <input.fl>", "")
                .replace("Paths need not be UTF-8. Use -- before a source path beginning with a dash.",
                         "Package entry paths need not be UTF-8.")))
        }
        _ => Err(format!("unknown command path '{}'", path.join(" "))),
    }
}
