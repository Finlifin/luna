use nir::program::{Expr, ExprKind, Literal, Package, SourceRange, Static};
use nir::{DeclarationId, Primitive, Term};

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
fn nir_static_and_package_display_emit_valid_sexpr() {
    let bare = Static {
        declaration: DeclarationId {
            package: 2,
            local: 10,
        },
        name: "counter\"one".into(),
        ty: Term::Primitive(Primitive::U64),
        initializer: None,
    };
    let bare_text = bare.to_string();
    assert!(balanced_sexpr(&bare_text), "{bare_text}");
    assert_eq!(
        bare_text,
        r#"(static (id (declaration (package 2) (local 10))) (name "counter\"one") (type (primitive u64)))"#
    );

    let initialized = Static {
        declaration: DeclarationId {
            package: 2,
            local: 11,
        },
        name: "flag".into(),
        ty: Term::Primitive(Primitive::Bool),
        initializer: Some(Expr {
            source: SourceRange { start: 1, end: 5 },
            ty: Term::Primitive(Primitive::Bool),
            kind: ExprKind::Literal(Literal::Bool(true)),
        }),
    };
    let init_text = initialized.to_string();
    assert!(balanced_sexpr(&init_text), "{init_text}");
    assert!(
        init_text.contains("(initializer (expr (type (primitive bool)) (source 1 5) (bool true)))"),
        "{init_text}"
    );

    let empty_pkg = Package::default();
    let empty_dump = empty_pkg.to_string();
    assert!(balanced_sexpr(&empty_dump), "{empty_dump}");
    assert!(!empty_dump.contains("(statics"), "{empty_dump}");

    let pkg = Package {
        statics: vec![bare, initialized],
        ..Default::default()
    };
    let dump = pkg.to_string();
    assert!(balanced_sexpr(&dump), "{dump}");
    assert!(
        dump.contains("(statics (static (id (declaration (package 2) (local 10)))"),
        "{dump}"
    );
    assert!(
        dump.contains("(static (id (declaration (package 2) (local 11)))"),
        "{dump}"
    );
}
