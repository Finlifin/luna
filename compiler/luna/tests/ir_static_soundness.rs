//! Integration tests for NIR/IIR S-expression dumps and static soundness across materialization.
#[path = "support/execution.rs"]
mod execution;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn balanced_sexpr(text: &str) -> bool {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0 && !in_string
}

#[test]
fn nir_and_iir_dumps_emit_statics_and_materialize_unreferenced_static_struct() {
    let source = r#"
        typealias I = .i64'builtin;

        struct HiddenConfig {
            enabled: bool,
            limit: I,
        }

        struct UnreferencedConfig {
            active: bool,
        }

        static config: HiddenConfig = HiddenConfig {
            enabled: true,
            limit: 42,
        };

        static unreferenced: UnreferencedConfig = UnreferencedConfig {
            active: false,
        };

        fn main() -> bool {
            let p = config.ref;
            p.*.enabled
        }
    "#;
    let package = execution::compile(source);
    let nir_dump = package.to_string();
    assert!(balanced_sexpr(&nir_dump), "unbalanced NIR:\n{nir_dump}");
    assert!(nir_dump.contains("(statics (static"), "{nir_dump}");
    assert!(nir_dump.contains("(name \"config\")"), "{nir_dump}");
    assert!(nir_dump.contains("(name \"unreferenced\")"), "{nir_dump}");

    let iir_graph = luna::instantiation::instantiate(&package, "main", 100_000).unwrap();
    assert_eq!(iir_graph.statics.len(), 2);
    assert!(iir_graph.nominals.iter().any(
        |n| matches!(&n.definition, iir::NominalDefinition::Struct(s) if s.name == "HiddenConfig")
    ));
    assert!(iir_graph.nominals.iter().any(
        |n| matches!(&n.definition, iir::NominalDefinition::Struct(s) if s.name == "UnreferencedConfig")
    ));
    iir_graph.validate(&mut 100_000).unwrap();

    let iir_dump = iir_graph.to_string();
    assert!(balanced_sexpr(&iir_dump), "unbalanced IIR:\n{iir_dump}");
    assert!(iir_dump.contains("(statics\n (static"), "{iir_dump}");
    assert!(iir_dump.contains("(name \"config\")"), "{iir_dump}");
    assert!(iir_dump.contains("(name \"unreferenced\")"), "{iir_dump}");
}

#[test]
fn cli_emit_nir_includes_statics_section() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "luna-ir-static-sexpr-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("source.fl");
    fs::write(
        &file,
        "typealias B = .bool'builtin;\nstatic flag: B = true;\nfn run() -> B { flag }\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&dir)
        .arg(&file)
        .args(["--emit", "nir"])
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "luna --emit nir failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(balanced_sexpr(&stdout), "unbalanced CLI NIR:\n{stdout}");
    assert!(stdout.contains("(statics (static"), "{stdout}");
    assert!(stdout.contains("(name \"flag\")"), "{stdout}");
}
