use hir::common::{Arg, FnParam, PathSegment, TyParam, TyParamKind};
use hir::expr::ClosureParam;
use hir::item::{FnModifiers, UseKind, UsePath};
use hir::pattern::BoundType;
use hir::*;
use rustc_span::{BytePos, DUMMY_SP, Span};
use symbol::{DefId, PathAnchor};

// Parse one complete S-expression, including escaped strings. Assertions below
// inspect the resulting relationships, rather than just counting parentheses.
#[derive(Debug, PartialEq)]
enum Sexp {
    Atom(String),
    Text(String),
    List(Vec<Sexp>),
}

impl Sexp {
    fn parse(text: &str) -> Self {
        fn read(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Sexp {
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            match chars.next().expect("missing expression") {
                '(' => {
                    let mut entries = Vec::new();
                    loop {
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                        if chars.peek() == Some(&')') {
                            chars.next();
                            break;
                        }
                        entries.push(read(chars));
                    }
                    Sexp::List(entries)
                }
                '"' => {
                    let mut text = String::new();
                    loop {
                        match chars.next().expect("unterminated string") {
                            '"' => break,
                            '\\' => text.push(match chars.next().expect("unfinished escape") {
                                'n' => '\n',
                                'r' => '\r',
                                't' => '\t',
                                'u' => {
                                    let hex: String = chars.take(4).collect();
                                    assert_eq!(hex.len(), 4);
                                    char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap()
                                }
                                '"' => '"',
                                '\\' => '\\',
                                other => panic!("unexpected escape: {other}"),
                            }),
                            c => {
                                assert!(!c.is_control(), "unescaped control in string");
                                text.push(c);
                            }
                        }
                    }
                    Sexp::Text(text)
                }
                ')' => panic!("extra closing parenthesis"),
                first => {
                    let mut atom = String::from(first);
                    while chars
                        .peek()
                        .is_some_and(|c| !c.is_whitespace() && !matches!(c, '(' | ')'))
                    {
                        let c = chars.next().unwrap();
                        assert!(!matches!(c, '"' | ';' | '\\'));
                        atom.push(c);
                    }
                    Sexp::Atom(atom)
                }
            }
        }
        let mut chars = text.chars().peekable();
        let value = read(&mut chars);
        assert!(chars.all(char::is_whitespace), "trailing data: {text}");
        value
    }

    fn list(&self) -> &[Self] {
        let Self::List(entries) = self else {
            panic!("expected list: {self:?}")
        };
        entries
    }

    fn tag(&self) -> &str {
        let Self::Atom(tag) = &self.list()[0] else {
            panic!("expected tag: {self:?}")
        };
        tag
    }

    fn field(&self, name: &str) -> &Self {
        let fields: Vec<_> = self.list()[1..]
            .iter()
            .filter(|entry| entry.tag() == name)
            .collect();
        assert_eq!(
            fields.len(),
            1,
            "field {name} missing or duplicated in {self:?}"
        );
        assert_eq!(fields[0].list().len(), 2);
        &fields[0].list()[1]
    }

    fn atom(&self, expected: impl ToString) {
        assert_eq!(self, &Self::Atom(expected.to_string()));
    }

    fn text(&self, expected: &str) {
        assert_eq!(self, &Self::Text(expected.to_owned()));
    }
}

fn owner(raw: u32) -> OwnerId {
    OwnerId::new(LocalDefId::new(raw))
}
fn id(local: u32) -> HirId {
    HirId::new(owner(7), ItemLocalId::new(local))
}
fn name(text: &str) -> Ident {
    Ident::new(Symbol::intern(text), DUMMY_SP)
}
fn expr(kind: ExprKind<'_>, local: u32) -> Expr<'_> {
    Expr {
        hir_id: id(local),
        kind,
        span: DUMMY_SP,
    }
}
fn pat(kind: PatternKind<'_>, local: u32) -> Pattern<'_> {
    Pattern {
        hir_id: id(local),
        kind,
        span: DUMMY_SP,
    }
}
fn parsed(value: &impl std::fmt::Display) -> Sexp {
    Sexp::parse(&value.to_string())
}

#[test]
fn contextual_code_dump_preserves_bindings_stage_and_substitution_direction() {
    let ty = expr(ExprKind::TyVoid, 200);
    let value = expr(
        ExprKind::ResolvedLocal(Symbol::intern("value"), id(201)),
        202,
    );
    let context = ExprKind::ContextualExprType(
        vec![hir::expr::CodeContextBinding {
            hir_id: id(203),
            name: name("port"),
            ty: &ty,
            default: Some(&value),
        }],
        &ty,
    );
    let dump = parsed(&context);
    assert_eq!(dump.tag(), "contextual-expr-type");
    let entry = dump.field("context");
    assert_eq!(entry.tag(), "binding");
    assert_id(entry.field("id"), 7, 203);
    assert_eq!(entry.field("default").tag(), "some");
    for kind in [
        ExprKind::CodeQuote(
            &value,
            vec![hir::expr::CodeSubstitution {
                port: name("port"),
                binding: &value,
            }],
        ),
        ExprKind::CodeSplice(
            &value,
            vec![hir::expr::CodeSubstitution {
                port: name("port"),
                binding: &value,
            }],
        ),
    ] {
        let dump = parsed(&kind);
        assert_eq!(dump.field("body").tag(), "expr");
        let substitution = dump.field("substitutions");
        assert_eq!(substitution.tag(), "substitute");
        assert_eq!(substitution.field("binding").tag(), "expr");
    }
}
fn assert_id(value: &Sexp, owner: u32, local: u32) {
    value.field("owner").atom(owner);
    value.field("local").atom(local);
}

#[test]
fn empty_and_invalid_packages_are_single_complete_expressions() {
    let mut package = Package::new();
    assert_eq!(
        package.to_string(),
        "(hir-package\n  (root (owner-id 4294967295))\n  (owners)\n  (bodies) (definitions) (pending-semantics))"
    );
    let item = Item {
        owner_id: owner(0),
        ident: name("bad ) ( name"),
        kind: ItemKind::Invalid,
        span: DUMMY_SP,
    };
    package.root_mod = owner(0);
    package.insert_owner(
        owner(0),
        OwnerInfo {
            node: OwnerNode::Item(&item),
            nodes: OwnerNodes::new(),
        },
    );
    let output = package.to_string();
    assert_eq!(output, package.dump_to_lisp());
    let tree = Sexp::parse(&output);
    assert_eq!(tree.tag(), "hir-package");
    let item = tree.list()[2].list()[1].field("node");
    assert_eq!(item.field("kind").tag(), "invalid");
    item.field("name").field("name").text("bad ) ( name");
    assert_id(item.field("id"), 0, 0);
}

#[test]
fn strings_symbols_identifiers_and_characters_escape_without_changing_contents() {
    let text = "paren () quote \" slash \\ newline\nreturn\rtab\t nul\0 escape\u{1b} delete\u{7f} 中文 🦀 ;|";
    for kind in [
        LitKind::String(text.into()),
        LitKind::Symbol(Symbol::intern(text)),
    ] {
        let output = kind.to_string();
        assert!(!output.contains('\n'));
        assert!(!output.contains('\0'));
        parsed(&kind).field("value").text(text);
        assert!(output.contains("\\u0000"));
        assert!(output.contains("\\u001b"));
    }
    for c in ['"', '\\', '\n', '\r', '\t', '\0', '\u{7f}', '中', '🦀', ')'] {
        parsed(&LitKind::Char(c))
            .field("value")
            .text(&c.to_string());
    }
    for kind in [
        ExprKind::UnresolvedIdent(Symbol::intern(text)),
        ExprKind::Builtin(Symbol::intern(text)),
    ] {
        parsed(&kind).field("name").text(text);
    }
    parsed(&PatternKind::Binding(BindingMode::ByRef, name(text), None))
        .field("name")
        .field("name")
        .text(text);
    parsed(&LitKind::Bool(true)).field("value").atom(true);
}

#[test]
fn numeral_dumps_preserve_source_spelling_without_evaluating_it() {
    for (kind, tag, spelling) in [
        (
            LitKind::Integer("1844674407370955161600000000000000000000".into()),
            "integer",
            "1844674407370955161600000000000000000000",
        ),
        (
            LitKind::Integer("0b1010__1010".into()),
            "integer",
            "0b1010__1010",
        ),
        (LitKind::Integer("0o000_777".into()), "integer", "0o000_777"),
        (LitKind::Integer("0xFF_FF".into()), "integer", "0xFF_FF"),
        (
            LitKind::Float("0.000_000_000_000_001".into()),
            "float",
            "0.000_000_000_000_001",
        ),
        (
            LitKind::Float("1.234567890123456789E+99999".into()),
            "float",
            "1.234567890123456789E+99999",
        ),
        (LitKind::Float("0.0e-99999".into()), "float", "0.0e-99999"),
    ] {
        let tree = parsed(&kind);
        assert_eq!(tree.tag(), tag);
        tree.field("spelling").text(spelling);
        assert_eq!(
            kind.to_string(),
            format!("({tag} (spelling \"{spelling}\"))")
        );
    }
}

#[test]
fn bodies_and_owners_are_ordered_by_complete_ids_and_only_expanded_in_their_tables() {
    let value = expr(ExprKind::Item(owner(7)), 9);
    let mut one = Package::new();
    let mut two = Package::new();
    let ids = [(9, 1), (7, 5), (7, 1), (8, 0)];
    for (o, l) in ids {
        one.insert_body(
            BodyId::new(HirId::new(owner(o), ItemLocalId::new(l))),
            Body {
                params: &[],
                value: &value,
            },
        );
    }
    for (o, l) in ids.into_iter().rev() {
        two.insert_body(
            BodyId::new(HirId::new(owner(o), ItemLocalId::new(l))),
            Body {
                params: &[],
                value: &value,
            },
        );
    }
    let items = [
        Item {
            owner_id: owner(7),
            ident: name("seven"),
            kind: ItemKind::Mod(ModDef {
                items: vec![owner(9), owner(7)],
            }),
            span: DUMMY_SP,
        },
        Item {
            owner_id: owner(9),
            ident: name("nine"),
            kind: ItemKind::Invalid,
            span: DUMMY_SP,
        },
    ];
    for item in &items {
        let mut nodes = OwnerNodes::new();
        nodes.insert(ItemLocalId::new(9), ItemLocalId::ZERO, Node::Expr(&value));
        one.insert_owner(
            item.owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes,
            },
        );
    }
    for item in items.iter().rev() {
        let mut nodes = OwnerNodes::new();
        nodes.insert(ItemLocalId::new(9), ItemLocalId::ZERO, Node::Expr(&value));
        two.insert_owner(
            item.owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes,
            },
        );
    }
    assert_eq!(one.to_string(), two.to_string());
    let tree = parsed(&one);
    let owners = &tree.list()[2].list()[1..];
    assert_eq!(owners.len(), 2);
    owners[0].field("id").list()[1].atom(7);
    owners[1].field("id").list()[1].atom(9);
    let index = &owners[0].list()[3].list()[1];
    assert_id(index.field("id"), 7, 9);
    assert_id(index.field("parent"), 7, 0);
    let bodies = &tree.list()[3].list()[1..];
    assert_eq!(bodies.len(), 4);
    for (entry, (o, l)) in bodies.iter().zip([(7, 1), (7, 5), (8, 0), (9, 1)]) {
        assert_id(entry.field("id"), o, l);
        let kind = entry.field("value").field("value").field("kind");
        assert_eq!(kind.tag(), "item-ref");
        kind.field("owner").list()[1].atom(7);
    }
}

#[test]
fn every_expression_variant_keeps_its_tag_and_payload() {
    let a = expr(
        ExprKind::Lit(Lit {
            kind: LitKind::Integer("23".into()),
            span: DUMMY_SP,
        }),
        11,
    );
    let b = expr(ExprKind::TyType, 12);
    let wild = pat(PatternKind::Wild, 13);
    let block = Block {
        hir_id: id(14),
        stmts: std::slice::from_ref(&a),
        expr: Some(&b),
        span: DUMMY_SP,
    };
    let args = [
        Arg::Positional(&a),
        Arg::Named(name("arg"), &b),
        Arg::Expand(&a),
        Arg::Implicit(&b),
        Arg::PostLambda(&a),
    ];
    let fields = [FieldExpr {
        ident: name("field"),
        expr: &b,
        span: DUMMY_SP,
    }];
    let arms = [PatternArm {
        guard: Some(&b),
        hir_id: id(15),
        pat: wild.clone(),
        body: &a,
        span: DUMMY_SP,
    }];
    let conds = [
        CondictionArm {
            hir_id: id(16),
            cond: Some(&a),
            body: &b,
            span: DUMMY_SP,
        },
        CondictionArm {
            hir_id: id(116),
            cond: None,
            body: &a,
            span: DUMMY_SP,
        },
    ];
    let params = [ClosureParam {
        hir_id: id(17),
        pat: wild.clone(),
        ty: Some(&b),
        span: DUMMY_SP,
    }];
    let typarams = [TyParam::new(id(18), TyParamKind::Positional(&b), DUMMY_SP).with_comptime()];
    let decl = LetDecl {
        comptime: false,
        mutable: false,
        hir_id: id(19),
        pattern: Pattern {
            hir_id: id(19),
            kind: PatternKind::Binding(BindingMode::ByValue, name("local"), None),
            span: DUMMY_SP,
        },
        guard: None,
        otherwise: None,
        ty: Some(&b),
        init: Some(&a),
        span: DUMMY_SP,
    };
    let body_id = BodyId::new(id(20));
    let segments = [PathSegment {
        ident: name("Type"),
        args: &args,
    }];
    let path = Path {
        anchor: PathAnchor::Super(2),
        segments: &segments,
        span: DUMMY_SP,
        res: Some(DefId::new(4, 8)),
    };
    // Every non-unit case identifies fields whose values must survive verbatim.
    let a_tree = parsed(&a);
    let b_tree = parsed(&b);
    let cases: Vec<(ExprKind<'_>, &str, Vec<(&str, &Sexp)>)> = vec![
        (
            ExprKind::Lit(Lit {
                kind: LitKind::Integer("23".into()),
                span: DUMMY_SP,
            }),
            "literal",
            vec![],
        ),
        (
            ExprKind::LiteralConversion {
                literal: &a,
                suffix: name("u32px"),
            },
            "literal-conversion",
            vec![("literal", &a_tree)],
        ),
        (ExprKind::Path(path), "path-expr", vec![]),
        (ExprKind::TacticParameter(3), "tactic-parameter", vec![]),
        (
            ExprKind::TacticArgument(&a),
            "tactic-argument",
            vec![("value", &a_tree)],
        ),
        (
            ExprKind::PostLambdaBody(body_id),
            "post-lambda-body",
            vec![],
        ),
        (
            ExprKind::UnresolvedIdent(Symbol::intern("lost")),
            "unresolved-ident",
            vec![],
        ),
        (
            ExprKind::ResolvedLocal(Symbol::intern("local"), id(41)),
            "resolved-local",
            vec![],
        ),
        (
            ExprKind::ResolvedFnParam(Symbol::intern("param"), id(42)),
            "resolved-fn-param",
            vec![],
        ),
        (
            ExprKind::ResolvedClauseParam(Symbol::intern("clause"), DefId::new(5, 43)),
            "resolved-clause-param",
            vec![],
        ),
        (
            ExprKind::ResolvedDef {
                name: Symbol::intern("Def"),
                has_clause_param: true,
                def_id: DefId::new(6, 44),
            },
            "resolved-def",
            vec![],
        ),
        (
            ExprKind::ResolvedLoopLabel(Symbol::intern("label"), id(45)),
            "resolved-loop-label",
            vec![],
        ),
        (ExprKind::SelfValue, "self-value", vec![]),
        (
            ExprKind::Index(&a, &b),
            "index",
            vec![("base", &a_tree), ("index", &b_tree)],
        ),
        (
            ExprKind::Application(&a, &args),
            "application",
            vec![("callee", &a_tree)],
        ),
        (
            ExprKind::CollectedProperties(&fields),
            "collected-properties",
            vec![],
        ),
        (
            ExprKind::ExtendedApplication(&a, &args),
            "extended-application",
            vec![("callee", &a_tree)],
        ),
        (
            ExprKind::NFApplication(&a, &args),
            "nf-application",
            vec![("callee", &a_tree)],
        ),
        (
            ExprKind::Binary(BinOp::Add, &a, &b),
            "binary",
            vec![("left", &a_tree), ("right", &b_tree)],
        ),
        (
            ExprKind::Unary(UnOp::Not, &a),
            "unary",
            vec![("value", &a_tree)],
        ),
        (
            ExprKind::If(&a, &block, Some(&b)),
            "if",
            vec![("condition", &a_tree)],
        ),
        (ExprKind::When(&conds), "when", vec![]),
        (ExprKind::Block(&block), "block-expr", vec![]),
        (ExprKind::Loop(&block), "loop", vec![]),
        (
            ExprKind::For {
                label: Some(name("outer")),
                pat: &wild,
                guard: Some(&b),
                iter: &a,
                body: &block,
            },
            "for",
            vec![("iterable", &a_tree)],
        ),
        (
            ExprKind::Match(&a, &arms),
            "match",
            vec![("value", &a_tree)],
        ),
        (
            ExprKind::Assign(&a, &b),
            "assign",
            vec![("place", &a_tree), ("value", &b_tree)],
        ),
        (
            ExprKind::AssignOp(BinOp::Mul, &a, &b),
            "assign-op",
            vec![("place", &a_tree), ("value", &b_tree)],
        ),
        (ExprKind::Return(Some(&a)), "return", vec![]),
        (
            ExprKind::Assumes(&a),
            "assumes",
            vec![("condition", &a_tree)],
        ),
        (ExprKind::Resume(Some(&a)), "resume", vec![]),
        (ExprKind::Break(name("label")), "break", vec![]),
        (ExprKind::Continue(name("label")), "continue", vec![]),
        (
            ExprKind::Projection(&a, name("field")),
            "projection",
            vec![("base", &a_tree)],
        ),
        (ExprKind::Tuple(std::slice::from_ref(&a)), "tuple", vec![]),
        (ExprKind::List(std::slice::from_ref(&a)), "list", vec![]),
        (
            ExprKind::Object(std::slice::from_ref(&a), &fields),
            "object",
            vec![],
        ),
        (ExprKind::Ref(&a), "ref", vec![("value", &a_tree)]),
        (ExprKind::Deref(&a), "deref", vec![("value", &a_tree)]),
        (
            ExprKind::ErrorNew(&a),
            "error-new",
            vec![("value", &a_tree)],
        ),
        (
            ExprKind::Closure(&params, Some(&b), body_id),
            "closure",
            vec![],
        ),
        (
            ExprKind::Cast(&a, &b),
            "cast",
            vec![("value", &a_tree), ("type", &b_tree)],
        ),
        (ExprKind::Let(&decl), "let", vec![]),
        (ExprKind::Semi(&a), "semi", vec![("value", &a_tree)]),
        (ExprKind::Item(owner(8)), "item-ref", vec![]),
        (ExprKind::Undefined, "undefined", vec![]),
        (ExprKind::Null, "null", vec![]),
        (ExprKind::Unit, "unit", vec![]),
        (
            ExprKind::InlineIf {
                cond: &a,
                then_expr: &b,
                else_expr: Some(&a),
            },
            "inline-if",
            vec![("condition", &a_tree), ("then", &b_tree)],
        ),
        (ExprKind::InlineMatch(&arms), "inline-match", vec![]),
        (
            ExprKind::EffectRequest(&a),
            "effect-request",
            vec![("computation", &a_tree)],
        ),
        (
            ExprKind::EffectHandle(&a, &arms),
            "effect-handle",
            vec![("computation", &a_tree)],
        ),
        (
            ExprKind::InlineFor {
                label: Some(name("loop")),
                pat: &wild,
                iter: &a,
                body: &b,
            },
            "inline-for",
            vec![("iterator", &a_tree), ("body", &b_tree)],
        ),
        (ExprKind::TyPtr(&b), "ty-ptr", vec![("type", &b_tree)]),
        (
            ExprKind::TyOptional(&b),
            "ty-optional",
            vec![("type", &b_tree)],
        ),
        (ExprKind::TyFn(&typarams), "ty-fn", vec![]),
        (ExprKind::TyNFFn(&typarams), "ty-nf-fn", vec![]),
        (
            ExprKind::TyFnArrow(&a, &b),
            "ty-fn-arrow",
            vec![("params", &a_tree), ("return-type", &b_tree)],
        ),
        (ExprKind::ReachabilityType, "reachability-type", vec![]),
        (ExprKind::ErrorQualifiedType, "error-qualified-type", vec![]),
        (
            ExprKind::EffectQualifiedType(&a, &b),
            "effect-qualified-type",
            vec![("effects", &a_tree), ("result", &b_tree)],
        ),
        (ExprKind::TyPlaceholder, "ty-placeholder", vec![]),
        (ExprKind::TyNoReturn, "ty-no-return", vec![]),
        (ExprKind::TyVoid, "ty-void", vec![]),
        (ExprKind::TyAny, "ty-any", vec![]),
        (ExprKind::TyType, "ty-type", vec![]),
        (ExprKind::TySelf, "ty-self", vec![]),
        (ExprKind::TermTypedWith, "term-typed-with", vec![]),
        (
            ExprKind::TraitBound(&a, &b),
            "trait-bound",
            vec![("subject", &a_tree), ("interface", &b_tree)],
        ),
        (ExprKind::LambdaBound, "lambda-bound", vec![]),
        (ExprKind::TermTraitBound, "term-trait-bound", vec![]),
        (ExprKind::Implication, "implication", vec![]),
        (ExprKind::Subtype, "subtype", vec![]),
        (ExprKind::Forall, "forall", vec![]),
        (ExprKind::Exist, "exist", vec![]),
        (ExprKind::Builtin(Symbol::intern("size")), "builtin", vec![]),
        (
            ExprKind::TakeBitVec(&a),
            "take-bit-vec",
            vec![("value", &a_tree)],
        ),
        (ExprKind::TypeOf(&a), "type-of", vec![("value", &a_tree)]),
        (
            ExprKind::TakeTagName(&a),
            "take-tag-name",
            vec![("value", &a_tree)],
        ),
        (
            ExprKind::MultiDeref(&a),
            "multi-deref",
            vec![("value", &a_tree)],
        ),
        (ExprKind::Invalid, "invalid", vec![]),
    ];
    assert_eq!(cases.len(), 81);
    for (kind, tag, children) in cases {
        let tree = parsed(&kind);
        assert_eq!(tree.tag(), tag);
        for (field, expected) in children {
            assert_eq!(tree.field(field), expected, "{tag}.{field}");
        }
        match tag {
            "literal-conversion" => tree.field("suffix").field("name").text("u32px"),
            "resolved-local" => assert_id(tree.field("target"), 7, 41),
            "resolved-fn-param" => assert_id(tree.field("target"), 7, 42),
            "resolved-loop-label" => assert_id(tree.field("target"), 7, 45),
            "resolved-clause-param" => {
                tree.field("target").field("package").atom(5);
                tree.field("target").field("index").atom(43);
            }
            "resolved-def" => {
                tree.field("target").field("package").atom(6);
                tree.field("target").field("index").atom(44);
                tree.field("has-clause-param").atom(true);
            }
            "application" | "extended-application" | "nf-application" => {
                let args = tree.field("args").list();
                assert_eq!(
                    args.iter().map(Sexp::tag).collect::<Vec<_>>(),
                    ["positional", "named", "expand", "implicit", "post-lambda"]
                );
                assert_eq!(args[0].field("value"), &a_tree);
                assert_eq!(args[1].field("value"), &b_tree);
                args[1].field("name").field("name").text("arg");
                assert_eq!(args[2].field("value"), &a_tree);
                assert_eq!(args[3].field("value"), &b_tree);
                assert_eq!(args[4].field("value"), &a_tree);
            }
            "path-expr" => {
                let path = tree.field("path");
                path.field("anchor").field("levels").atom(2);
                path.field("resolution").list()[1].field("package").atom(4);
                path.field("resolution").list()[1].field("index").atom(8);
                assert_eq!(
                    path.field("segments").list()[0].field("args").list().len(),
                    5
                );
            }
            "block-expr" | "loop" => {
                let block = tree.field(if tag == "loop" { "body" } else { "block" });
                assert_id(block.field("id"), 7, 14);
                assert_eq!(
                    block.field("statements").list(),
                    std::slice::from_ref(&a_tree)
                );
                assert_eq!(&block.field("tail").list()[1], &b_tree);
            }
            "return" | "resume" => assert_eq!(&tree.field("value").list()[1], &a_tree),
            "tuple" | "list" => {
                assert_eq!(tree.field("values").list(), std::slice::from_ref(&a_tree))
            }
            "object" => {
                assert_eq!(tree.field("bases").list(), std::slice::from_ref(&a_tree));
                assert_eq!(tree.field("fields").list()[0].field("value"), &b_tree);
            }
            "tactic-parameter" => tree.field("index").atom(3),
            "post-lambda-body" => assert_id(tree.field("body"), 7, 20),
            "closure" => {
                assert_id(tree.field("body"), 7, 20);
                assert_eq!(&tree.field("return-type").list()[1], &b_tree);
                let param = &tree.field("params").list()[0];
                assert_id(param.field("id"), 7, 17);
                assert_eq!(&param.field("type").list()[1], &b_tree);
                assert_eq!(param.field("pattern").field("kind").tag(), "wild");
            }
            "when" => {
                let arm = &tree.field("arms").list()[0];
                assert_id(arm.field("id"), 7, 16);
                assert_eq!(&arm.field("condition").list()[1], &a_tree);
                assert_eq!(arm.field("body"), &b_tree);
                assert_eq!(
                    tree.field("arms").list()[1].field("condition").tag(),
                    "none"
                );
            }
            "match" | "inline-match" => {
                let arm = &tree.field("arms").list()[0];
                assert_id(arm.field("id"), 7, 15);
                assert_eq!(arm.field("body"), &a_tree);
                assert_eq!(arm.field("pattern").field("kind").tag(), "wild");
            }
            "let" => {
                let decl = tree.field("declaration");
                assert_id(decl.field("id"), 7, 19);
                assert_eq!(&decl.field("type").list()[1], &b_tree);
                assert_eq!(&decl.field("init").list()[1], &a_tree);
            }
            "ty-fn" | "ty-nf-fn" => {
                let param = &tree.field("params").list()[0];
                assert_id(param.field("id"), 7, 18);
                param.field("flags").atom(2);
                assert_eq!(param.field("kind").field("type"), &b_tree);
            }
            _ => {}
        }
    }
}

#[test]
fn every_pattern_variant_preserves_nested_patterns_and_range_boundaries() {
    let wild = pat(PatternKind::Wild, 1);
    let value = expr(ExprKind::Unit, 2);
    let fields = [FieldPat {
        ident: name("f"),
        pat: wild.clone(),
        span: DUMMY_SP,
    }];
    let kinds = [
        (PatternKind::Wild, "wild"),
        (PatternKind::Projection(&wild, name("field")), "projection"),
        (
            PatternKind::Binding(BindingMode::ByRef, name("binding"), Some(&wild)),
            "binding",
        ),
        (PatternKind::Const(&value), "const"),
        (PatternKind::Comptime(&value), "comptime"),
        (PatternKind::Tuple(std::slice::from_ref(&wild)), "tuple"),
        (PatternKind::Struct(&wild, &fields, true), "struct"),
        (
            PatternKind::List(std::slice::from_ref(&wild), Some(&wild)),
            "list",
        ),
        (
            PatternKind::AppTuple(&wild, std::slice::from_ref(&wild)),
            "app-tuple",
        ),
        (PatternKind::AppStruct(&wild, &fields), "app-struct"),
        (PatternKind::OptionSome(&wild), "option-some"),
        (PatternKind::OptionNull, "option-null"),
        (PatternKind::ErrorOk(&wild), "error-ok"),
        (PatternKind::ErrorErr(&wild), "error-err"),
        (PatternKind::Or(std::slice::from_ref(&wild)), "or"),
        (PatternKind::Ref(&wild), "ref"),
        (PatternKind::Not(&wild), "not"),
        (
            PatternKind::Range(Some(&value), None, BoundType::Inclusive),
            "range",
        ),
        (PatternKind::Async, "async"),
        (
            PatternKind::BitVec {
                radix: 2,
                segments: vec![],
            },
            "bit-vec",
        ),
        (PatternKind::Invalid, "invalid"),
    ];
    assert_eq!(kinds.len(), 21);
    for (kind, tag) in kinds {
        let tree = parsed(&kind);
        assert_eq!(tree.tag(), tag);
        match tag {
            "projection" | "app-tuple" | "app-struct" | "struct" => {
                assert_eq!(tree.field("base"), &parsed(&wild))
            }
            "option-some" | "error-ok" | "error-err" | "ref" | "not" => {
                assert_eq!(tree.field("pattern"), &parsed(&wild))
            }
            "const" | "comptime" => assert_eq!(tree.field("value"), &parsed(&value)),
            "binding" => {
                tree.field("mode").atom("by-ref");
                assert_eq!(&tree.field("pattern").list()[1], &parsed(&wild));
            }
            "range" => {
                assert_eq!(&tree.field("start").list()[1], &parsed(&value));
                assert_eq!(tree.field("end").tag(), "none");
                tree.field("bound").atom("inclusive");
            }
            _ => {}
        }
        if matches!(tag, "tuple" | "list" | "app-tuple" | "or") {
            assert_eq!(tree.field("patterns").list(), [parsed(&wild)]);
        }
        if matches!(tag, "struct" | "app-struct") {
            let field = &tree.field("fields").list()[0];
            field.field("name").field("name").text("f");
            assert_eq!(field.field("pattern"), &parsed(&wild));
        }
        if tag == "struct" {
            tree.field("rest").atom(true);
        }
        if tag == "list" {
            assert_eq!(&tree.field("rest").list()[1], &parsed(&wild));
        }
    }
    let range = parsed(&PatternKind::Range(
        None,
        Some(&value),
        BoundType::Exclusive,
    ));
    assert_eq!(range.field("start").tag(), "none");
    assert_eq!(&range.field("end").list()[1], &parsed(&value));
    range.field("bound").atom("exclusive");
    parsed(&PatternKind::Binding(BindingMode::ByValue, name("x"), None))
        .field("mode")
        .atom("by-value");
}

#[test]
fn function_signatures_preserve_parameters_flags_clauses_return_binding_and_body() {
    let ty = expr(ExprKind::TyType, 50);
    let default = expr(
        ExprKind::Lit(Lit {
            kind: LitKind::Integer("37".into()),
            span: DUMMY_SP,
        }),
        51,
    );
    let kinds = [
        TyParamKind::PositionalDependencyCatched(name("Captured"), &ty),
        TyParamKind::Positional(&ty),
        TyParamKind::Optional(name("optional"), &ty, &default),
        TyParamKind::Varadic(name("rest"), &ty),
        TyParamKind::Itself { is_ref: true },
    ];
    let params: Vec<_> = kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| FnParam {
            hir_id: id(i as u32 + 1),
            name: name(&format!("p{i}")),
            index: i as u32,
            ty: TyParam::new(id(i as u32 + 11), kind, DUMMY_SP).with_flags(0x8000003f),
            span: DUMMY_SP,
        })
        .collect();
    let clause_kinds = [
        ClauseParamKind::Type(name("TypeName")),
        ClauseParamKind::Positional(name("position"), &ty),
        ClauseParamKind::Optional(name("option"), &ty, &default),
        ClauseParamKind::Varadic(name("args"), &ty),
        ClauseParamKind::Quote(Box::new(ClauseParamKind::Positional(name("quoted"), &ty))),
    ];
    let clauses: Vec<_> = clause_kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| ClauseParam {
            definition: None,
            hir_id: id(i as u32 + 21),
            name: name(&format!("clause{i}")),
            kind,
            span: DUMMY_SP,
        })
        .collect();
    let constraints: Vec<_> = [
        ClauseConstraintKind::Requires(&default),
        ClauseConstraintKind::Ensures(&default),
        ClauseConstraintKind::Decreases(&default),
        ClauseConstraintKind::Outcome,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, kind)| ClauseConstraint {
        label: None,
        hir_id: id(i as u32 + 31),
        kind,
        span: DUMMY_SP,
    })
    .collect();
    let sig = FnSig {
        params: &params,
        return_ty: Some(&ty),
        return_bind: Some(name("result")),
        return_binding: None,
        modifiers: FnModifiers {
            is_inline: true,
            is_quote: false,
            is_normal_form_head: false,
            is_pure: true,
            is_comptime: true,
            is_extern: true,
        },
        clause_params: &clauses,
        clause_constraints: &constraints,
        span: Span::new(BytePos(4), BytePos(90)),
    };
    let body_id = BodyId::new(id(0));
    let item = Item {
        owner_id: owner(7),
        ident: name("f"),
        kind: ItemKind::Fn(sig.clone(), body_id),
        span: DUMMY_SP,
    };
    let body_params: Vec<_> = params.iter().map(BodyParam::from_fn_param).collect();
    let mut package = Package::new();
    package.insert_owner(
        owner(7),
        OwnerInfo {
            node: OwnerNode::Item(&item),
            nodes: OwnerNodes::new(),
        },
    );
    package.insert_body(
        body_id,
        Body {
            params: &body_params,
            value: &default,
        },
    );
    let tree = parsed(&package);
    let function = tree.list()[2].list()[1].field("node").field("kind");
    assert_eq!(function.tag(), "fn");
    assert_id(function.field("body"), 7, 0);
    let signature = function.field("signature");
    signature.field("span").field("lo").atom(4);
    signature.field("span").field("hi").atom(90);
    assert_eq!(&signature.field("return-type").list()[1], &parsed(&ty));
    signature.field("return-bind").list()[1]
        .field("name")
        .text("result");
    for flag in ["inline", "pure", "comptime", "extern"] {
        signature.field("modifiers").field(flag).atom(true);
    }
    let types = [
        "positional-dependency-catched",
        "positional",
        "optional",
        "variadic",
        "itself",
    ];
    for (i, param) in signature.field("params").list().iter().enumerate() {
        assert_id(param.field("id"), 7, i as u32 + 1);
        param.field("index").atom(i);
        param.field("name").field("name").text(&format!("p{i}"));
        let tparam = param.field("type");
        assert_id(tparam.field("id"), 7, i as u32 + 11);
        tparam.field("flags").atom(0x8000003fu32);
        assert_eq!(tparam.field("kind").tag(), types[i]);
        if i < 4 {
            assert_eq!(tparam.field("kind").field("type"), &parsed(&ty));
        }
        if i == 2 {
            assert_eq!(tparam.field("kind").field("default"), &parsed(&default));
        }
        if i == 4 {
            tparam.field("kind").field("ref").atom(true);
        }
    }
    let clause_tags = ["type", "positional", "optional", "variadic", "quote"];
    for (i, clause) in signature.field("clause-params").list().iter().enumerate() {
        assert_id(clause.field("id"), 7, i as u32 + 21);
        assert_eq!(clause.field("kind").tag(), clause_tags[i]);
        clause
            .field("name")
            .field("name")
            .text(&format!("clause{i}"));
        if i != 0 {
            let kind = clause.field("kind");
            let typed = if i == 4 {
                kind.field("parameter")
            } else {
                kind
            };
            assert_eq!(typed.field("type"), &parsed(&ty));
        }
        if i == 2 {
            assert_eq!(clause.field("kind").field("default"), &parsed(&default));
        }
    }
    let constraint_tags = ["requires", "ensures", "decreases", "outcome"];
    for (i, constraint) in signature.field("constraints").list().iter().enumerate() {
        assert_id(constraint.field("id"), 7, i as u32 + 31);
        assert_eq!(constraint.field("kind").tag(), constraint_tags[i]);
        if i != 3 {
            assert_eq!(constraint.field("kind").field("value"), &parsed(&default));
        }
    }
    let body = tree.list()[3].list()[1].field("value");
    assert_eq!(body.field("value"), &parsed(&default));
    assert_eq!(body.field("params").list().len(), 5);
    for (i, param) in body.field("params").list().iter().enumerate() {
        assert_id(param.field("id"), 7, i as u32 + 1);
        assert_eq!(
            param.field("type"),
            signature.field("params").list()[i].field("type")
        );
    }
    let nf = NFSig {
        params: &params,
        return_ty: Some(&ty),
        return_bind: Some(name("result")),
        clause_params: &clauses,
        clause_constraints: &constraints,
        span: DUMMY_SP,
    };
    let nf = parsed(&nf);
    assert_eq!(nf.tag(), "nf-signature");
    for field in [
        "params",
        "return-type",
        "return-bind",
        "clause-params",
        "constraints",
    ] {
        assert_eq!(nf.field(field), signature.field(field));
    }
}

#[test]
fn item_definitions_include_fields_variants_nested_owners_and_import_styles() {
    let ty = expr(ExprKind::TyAny, 1);
    let value = expr(
        ExprKind::Lit(Lit {
            kind: LitKind::Integer("99".into()),
            span: DUMMY_SP,
        }),
        2,
    );
    let wild = pat(PatternKind::Wild, 3);
    let pattern = pat(PatternKind::OptionSome(&wild), 4);
    let fields = [FieldDef {
        hir_id: id(5),
        ident: name("field"),
        ty: &ty,
        default: Some(&value),
        span: DUMMY_SP,
    }];
    let unit = Variant {
        hir_id: id(6),
        ident: name("Unit"),
        kind: VariantKind::Unit,
        span: DUMMY_SP,
    };
    let variants: Vec<_> = [
        VariantKind::Unit,
        VariantKind::Pattern(&pattern),
        VariantKind::Const(&value),
        VariantKind::Tuple(std::slice::from_ref(&ty)),
        VariantKind::Struct(&fields),
        VariantKind::SubEnum(std::slice::from_ref(&unit)),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, kind)| Variant {
        hir_id: id(i as u32 + 10),
        ident: name(&format!("V{i}")),
        kind,
        span: DUMMY_SP,
    })
    .collect();
    let clauses = [ClauseParam {
        definition: None,
        hir_id: id(20),
        name: name("T"),
        kind: ClauseParamKind::Type(name("T")),
        span: DUMMY_SP,
    }];
    let constraints = [ClauseConstraint {
        label: None,
        hir_id: id(21),
        kind: ClauseConstraintKind::Requires(&value),
        span: DUMMY_SP,
    }];
    let segments = [PathSegment {
        ident: name("source"),
        args: &[],
    }];
    let path = Path {
        anchor: PathAnchor::Package,
        segments: &segments,
        span: DUMMY_SP,
        res: None,
    };
    let kinds = [
        (
            ItemKind::Struct(StructDef {
                fields: &fields,
                clause_params: &clauses,
                clause_constraints: &constraints,
                nested_items: vec![owner(3)],
            }),
            "struct",
        ),
        (
            ItemKind::Enum(EnumDef {
                variants: &variants,
                configurations: vec![],
                clause_params: &clauses,
                clause_constraints: &constraints,
                nested_items: vec![owner(4)],
            }),
            "enum",
        ),
        (
            ItemKind::Mod(ModDef {
                items: vec![owner(2), owner(1)],
            }),
            "mod",
        ),
        (
            ItemKind::Impl(ImplDef {
                self_ty: &ty,
                trait_ref: Some(&value),
                clause_params: &clauses,
                clause_constraints: &constraints,
                items: vec![owner(5)],
            }),
            "impl",
        ),
        (
            ItemKind::Trait(TraitDef {
                super_traits: None,
                clause_params: &clauses,
                clause_constraints: &constraints,
                items: vec![owner(6)],
            }),
            "trait",
        ),
        (ItemKind::TypeAlias(&ty, &[], &[]), "type-alias"),
        (ItemKind::Newtype(&ty, &clauses, &constraints), "newtype"),
        (
            ItemKind::Use(UsePath {
                path: path.clone(),
                kind: UseKind::Glob,
                span: DUMMY_SP,
            }),
            "use",
        ),
        (ItemKind::Const(&ty, &value), "const"),
        (ItemKind::Invalid, "invalid"),
    ];
    for (kind, tag) in kinds {
        let tree = parsed(&kind);
        assert_eq!(tree.tag(), tag);
        if matches!(tag, "struct" | "enum" | "impl" | "trait") {
            let definition = tree.field("definition");
            assert_eq!(
                definition.field("clause-params").list(),
                [parsed(&clauses[0])]
            );
            assert_eq!(
                definition.field("constraints").list(),
                [parsed(&constraints[0])]
            );
        }
        match tag {
            "struct" => {
                let def = tree.field("definition");
                def.field("nested-items").list()[0].list()[1].atom(3);
                let field = &def.field("fields").list()[0];
                assert_id(field.field("id"), 7, 5);
                assert_eq!(field.field("type"), &parsed(&ty));
                assert_eq!(&field.field("default").list()[1], &parsed(&value));
            }
            "enum" => {
                let def = tree.field("definition");
                def.field("nested-items").list()[0].list()[1].atom(4);
                let tags = ["unit", "pattern", "const", "tuple", "struct", "sub-enum"];
                for (i, variant) in def.field("variants").list().iter().enumerate() {
                    assert_id(variant.field("id"), 7, i as u32 + 10);
                    variant.field("name").field("name").text(&format!("V{i}"));
                    let kind = variant.field("kind");
                    assert_eq!(kind.tag(), tags[i]);
                    match i {
                        1 => assert_eq!(kind.field("pattern"), &parsed(&pattern)),
                        2 => assert_eq!(kind.field("value"), &parsed(&value)),
                        3 => assert_eq!(kind.field("types").list(), [parsed(&ty)]),
                        4 => assert_eq!(kind.field("fields").list(), [parsed(&fields[0])]),
                        5 => assert_eq!(kind.field("variants").list(), [parsed(&unit)]),
                        _ => {}
                    }
                }
            }
            "impl" => {
                let def = tree.field("definition");
                assert_eq!(def.field("self-type"), &parsed(&ty));
                assert_eq!(&def.field("trait").list()[1], &parsed(&value));
                def.field("items").list()[0].list()[1].atom(5);
            }
            "trait" => tree.field("definition").field("items").list()[0].list()[1].atom(6),
            "mod" => {
                tree.field("items").list()[0].list()[1].atom(2);
                tree.field("items").list()[1].list()[1].atom(1);
            }
            "type-alias" => assert_eq!(tree.field("type"), &parsed(&ty)),
            "newtype" => {
                assert_eq!(tree.field("representation"), &parsed(&ty));
                assert_eq!(tree.field("parameters").list(), [parsed(&clauses[0])]);
                assert_eq!(tree.field("constraints").list(), [parsed(&constraints[0])]);
                assert_eq!(DefKind::from_item_kind(&kind), DefKind::Newtype);
            }
            "const" => {
                assert_eq!(tree.field("type"), &parsed(&ty));
                assert_eq!(tree.field("value"), &parsed(&value));
            }
            _ => {}
        }
    }
    let names = [name("a"), name("b")];
    let nested_paths = [UsePath {
        path: path.clone(),
        kind: UseKind::Alias(name("nested_alias")),
        span: DUMMY_SP,
    }];
    for (kind, tag) in [
        (UseKind::Group(&nested_paths), "group"),
        (UseKind::Simple, "simple"),
        (UseKind::Glob, "glob"),
        (UseKind::Multi(&names), "multi"),
        (UseKind::Alias(name("alias")), "alias"),
    ] {
        let tree = parsed(&UsePath {
            path: path.clone(),
            kind,
            span: DUMMY_SP,
        });
        assert_eq!(tree.field("path").field("anchor").tag(), "package");
        assert_eq!(tree.field("path").field("resolution").tag(), "none");
        let kind = tree.field("kind");
        assert_eq!(kind.tag(), tag);
        if tag == "group" {
            let paths = kind.field("paths").list();
            assert_eq!(paths.len(), 1);
            paths[0]
                .field("kind")
                .field("name")
                .field("name")
                .text("nested_alias");
        }
        if tag == "multi" {
            let values = kind.field("names").list();
            assert_eq!(values.len(), 2);
            values[0].field("name").text("a");
            values[1].field("name").text("b");
        }
        if tag == "alias" {
            kind.field("name").field("name").text("alias");
        }
    }
    let local = parsed(&ExprKind::Path(Path {
        anchor: PathAnchor::Local,
        ..path
    }));
    assert_eq!(local.field("path").field("anchor").tag(), "local");
}

#[test]
fn absent_optional_payloads_and_operator_spelling_are_explicit() {
    let value = expr(ExprKind::Unit, 1);
    for kind in [ExprKind::Return(None), ExprKind::Resume(None)] {
        assert_eq!(parsed(&kind).field("value").tag(), "none");
    }
    let block = Block {
        hir_id: id(2),
        stmts: &[],
        expr: None,
        span: DUMMY_SP,
    };
    assert_eq!(parsed(&block).field("tail").tag(), "none");
    assert_eq!(
        parsed(&ExprKind::If(&value, &block, None))
            .field("else")
            .tag(),
        "none"
    );
    let decl = LetDecl {
        comptime: false,
        mutable: false,
        hir_id: id(3),
        pattern: Pattern {
            hir_id: id(3),
            kind: PatternKind::Wild,
            span: DUMMY_SP,
        },
        guard: None,
        otherwise: None,
        ty: None,
        init: None,
        span: DUMMY_SP,
    };
    assert_eq!(parsed(&decl).field("type").tag(), "none");
    assert_eq!(parsed(&decl).field("init").tag(), "none");
    let sig = FnSig {
        params: &[],
        return_ty: None,
        return_bind: None,
        return_binding: None,
        modifiers: FnModifiers::default(),
        clause_params: &[],
        clause_constraints: &[],
        span: DUMMY_SP,
    };
    let signature = parsed(&sig);
    assert_eq!(signature.field("return-type").tag(), "none");
    assert_eq!(signature.field("return-bind").tag(), "none");
    for flag in ["inline", "pure", "comptime", "extern"] {
        signature.field("modifiers").field(flag).atom(false);
    }
    for (op, spelling) in [
        (BinOp::Add, "+"),
        (BinOp::Sub, "-"),
        (BinOp::Mul, "*"),
        (BinOp::Div, "/"),
        (BinOp::Rem, "%"),
        (BinOp::Eq, "=="),
        (BinOp::Ne, "!="),
        (BinOp::Lt, "<"),
        (BinOp::Gt, ">"),
        (BinOp::Le, "<="),
        (BinOp::Ge, ">="),
        (BinOp::And, "and"),
        (BinOp::Or, "or"),
    ] {
        parsed(&ExprKind::Binary(op, &value, &value))
            .field("op")
            .atom(spelling);
    }
    parsed(&ExprKind::Unary(UnOp::Neg, &value))
        .field("op")
        .atom("-");
    parsed(&ExprKind::Unary(UnOp::Not, &value))
        .field("op")
        .atom("not");
}

#[test]
fn collected_properties_dump_preserves_nested_fields_and_escaping() {
    let label = "key\"\\\n";
    let value = expr(
        ExprKind::Lit(Lit {
            kind: LitKind::String("value\"\\\n".into()),
            span: DUMMY_SP,
        }),
        901,
    );
    let inner_fields = [FieldExpr {
        ident: name(label),
        expr: &value,
        span: DUMMY_SP,
    }];
    let inner = expr(ExprKind::CollectedProperties(&inner_fields), 902);
    let outer_fields = [FieldExpr {
        ident: name("nested"),
        expr: &inner,
        span: DUMMY_SP,
    }];
    let outer = expr(ExprKind::CollectedProperties(&outer_fields), 903);
    let tree = parsed(&outer);
    let kind = tree.field("kind");
    assert_eq!(kind.tag(), "collected-properties");
    let nested = &kind.field("fields").list()[0];
    assert_eq!(nested.field("value"), &parsed(&inner));
    let inner_tree = parsed(&inner);
    let field = &inner_tree.field("kind").field("fields").list()[0];
    field.field("name").field("name").text(label);
    assert_eq!(field.field("value"), &parsed(&value));
    println!("PROPERTY_HIR_DUMP={outer}\nPROPERTY_HIR_END");
}
