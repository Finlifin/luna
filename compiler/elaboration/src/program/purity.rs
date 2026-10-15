//! Bounded program-behavior analysis, independent of PTC and interpretation.
//! Runtime `pure` promises require P4, while runtime defaults also require finite
//! completion. Recursion/loops alone do not violate P4; neither mode authorizes
//! unknown calls, external storage effects or unaccounted resource cleanup.
use super::*;
use crate::verification::memory::{CallableInput, CallablePlace};
use std::collections::{HashMap, HashSet};
fn code_root(binding: n::LocalId) -> CallablePlace {
    CallablePlace {
        binding,
        fields: vec![],
    }
}
// Shared owned-path encoding: a variant marker, its index, then payload field.
const VARIANT_FIELD: usize = usize::MAX;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CodeKey {
    Named(DeclarationId, Vec<Term>),
    Closure(usize),
}
type Inputs = BTreeMap<CallableInput, Vec<CodeKey>>;
#[derive(Clone, Debug, PartialEq)]
struct BehaviorCode {
    definition: n::Expr,
    captures: BTreeMap<CallablePlace, Vec<CodeKey>>,
}
impl BehaviorCode {
    fn plain(definition: n::Expr) -> Self {
        Self {
            definition,
            captures: BTreeMap::new(),
        }
    }
    fn charge(&self, fuel: &mut usize) -> bool {
        let mut tree_fuel = *fuel as u64;
        let valid =
            constants::charge_copy(&self.definition, &mut tree_fuel, Span::default()).is_ok();
        *fuel = tree_fuel as usize;
        if !valid {
            return false;
        }
        for (place, keys) in &self.captures {
            let Some(next) = fuel.checked_sub(place.fields.len().saturating_add(1)) else {
                *fuel = 0;
                return false;
            };
            *fuel = next;
            for key in keys {
                let Some(next) = fuel.checked_sub(1) else {
                    *fuel = 0;
                    return false;
                };
                *fuel = next;
                if let CodeKey::Named(_, types) = key {
                    for ty in types {
                        if crate::associated::charge_projection_copy(ty, fuel).is_err() {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}
impl std::ops::Deref for BehaviorCode {
    type Target = n::Expr;
    fn deref(&self) -> &n::Expr {
        &self.definition
    }
}
type Codes = Vec<BehaviorCode>;
enum ReturnedCodeSource<'a> {
    Named(DeclarationId, &'a [Term]),
    Closure(&'a BehaviorCode),
}
/// Bottom is a pending loop equation, never evidence that a variant is absent.
#[derive(Clone, Debug, PartialEq)]
enum LoopCode {
    Bottom,
    Known(Codes),
    Unknown,
}
impl std::fmt::Display for LoopCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bottom => f.write_str("(loop-code bottom)"),
            Self::Unknown => f.write_str("(loop-code unknown)"),
            Self::Known(codes) if codes.is_empty() => f.write_str("(loop-code absent-variant)"),
            Self::Known(codes) => {
                f.write_str("(loop-code (candidates")?;
                for code in codes {
                    write!(f, " {code}")?;
                }
                f.write_str("))")
            }
        }
    }
}
fn copy_loop_code(code: &LoopCode, fuel: &mut usize) -> Option<LoopCode> {
    *fuel = fuel.checked_sub(1)?;
    Some(match code {
        LoopCode::Known(codes) => LoopCode::Known(copy_codes(codes, fuel)?),
        LoopCode::Bottom => LoopCode::Bottom,
        LoopCode::Unknown => LoopCode::Unknown,
    })
}

fn copy_codes(codes: &Codes, fuel: &mut usize) -> Option<Codes> {
    for code in codes {
        if !code.charge(fuel) {
            return None;
        }
    }
    Some(codes.clone())
}
impl std::fmt::Display for CodeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closure(index) => write!(f, "(closure-key {index})"),
            Self::Named(declaration, arguments) => {
                write!(f, "(named-code (declaration {declaration}) (arguments")?;
                for ty in arguments {
                    write!(f, " {ty}")?;
                }
                f.write_str("))")
            }
        }
    }
}
impl std::fmt::Display for BehaviorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(behavior-code (definition {}) (captures",
            self.definition
        )?;
        for (binding, keys) in &self.captures {
            if keys.is_empty() && binding.fields.contains(&VARIANT_FIELD) {
                write!(f, " (capture (place {binding}) (absent-variant))")?;
                continue;
            }
            write!(f, " (capture (place {binding}) (candidates")?;
            for key in keys {
                write!(f, " {key}")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod behavior_code_snapshot_tests {
    use super::*;
    #[test]
    fn capture_initializers_keep_evaluated_code_before_later_updates() {
        let source = n::SourceRange { start: 0, end: 1 };
        let boolean = Term::Primitive(P::Bool);
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(boolean.clone()),
        };
        let good = DeclarationId {
            package: 0,
            local: 10,
        };
        let bad = DeclarationId {
            package: 0,
            local: 11,
        };
        let producer = DeclarationId {
            package: 0,
            local: 12,
        };
        let code = |id| n::Expr {
            source,
            ty: signature.clone(),
            kind: K::Function(id),
        };
        let truth = n::Expr {
            source,
            ty: boolean.clone(),
            kind: K::Literal(n::Literal::Bool(true)),
        };
        let function = |id, body: n::Expr| n::Function {
            declaration: id,
            name: format!("f{}", id.local),
            phase: n::FunctionPhase::Runtime,
            inline: false,
            pure: false,
            type_parameters: vec![],
            parameters: vec![],
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(body.ty.clone()),
            },
            body,
        };
        let producer_function = function(producer, code(good));
        let produced = n::Expr {
            source,
            ty: signature.clone(),
            kind: K::Call {
                callee: Box::new(n::Expr {
                    source,
                    ty: producer_function.signature.clone(),
                    kind: K::Function(producer),
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        };
        let mut package = n::Package::default();
        package.functions = vec![
            function(good, truth.clone()),
            function(
                bad,
                n::Expr {
                    source,
                    ty: boolean.clone(),
                    kind: K::Call {
                        callee: Box::new(code(bad)),
                        arguments: vec![],
                        parameter_bindings: vec![],
                    },
                },
            ),
            producer_function,
        ];
        let alias = n::LocalId { owner: 4, index: 0 };
        let spare = n::LocalId { owner: 4, index: 1 };
        let local = n::Expr {
            source,
            ty: signature.clone(),
            kind: K::Local(alias),
        };
        for finite in [false, true] {
            for first in [
                None,
                Some(Box::new(local.clone())),
                Some(Box::new(produced.clone())),
            ] {
                let closure = n::Expr {
                    source,
                    ty: Term::Closure {
                        environment: vec![signature.clone(), boolean.clone()],
                        signature: Box::new(signature.clone()),
                    },
                    kind: K::Closure {
                        parameters: vec![],
                        captures: vec![
                            n::Capture {
                                binding: alias,
                                ty: signature.clone(),
                                value: first,
                            },
                            n::Capture {
                                binding: spare,
                                ty: boolean.clone(),
                                value: Some(Box::new(n::Expr {
                                    source,
                                    ty: boolean.clone(),
                                    kind: K::Block {
                                        statements: vec![n::Expr {
                                            source,
                                            ty: Term::Primitive(P::Void),
                                            kind: K::Assign {
                                                place: n::Place {
                                                    root: alias,
                                                    fields: vec![],
                                                },
                                                value: Box::new(code(bad)),
                                            },
                                        }],
                                        result: Some(Box::new(truth.clone())),
                                    },
                                })),
                            },
                        ],
                        body: std::sync::Arc::new(n::Expr {
                            source,
                            ty: boolean.clone(),
                            kind: K::Call {
                                callee: Box::new(local.clone()),
                                arguments: vec![],
                                parameter_bindings: vec![],
                            },
                        }),
                    },
                };
                let mut analysis = Analysis::new(&package, finite);
                analysis
                    .callables
                    .insert(code_root(alias), vec![BehaviorCode::plain(code(good))]);
                assert!(analysis.expression(&closure, &mut BTreeSet::from([alias]), false, 0));
                let captured = analysis.result_code.take().unwrap().remove(0);
                assert_eq!(
                    captured.captures[&code_root(alias)],
                    vec![CodeKey::Named(good, vec![])]
                );
                assert!(analysis.call_candidate(&captured, &BTreeMap::new(), &truth, 0));
                if finite {
                    // An unused closure still evaluates its initializers;
                    // divergent initialization cannot inherit a finite body.
                    let mut divergent = closure.clone();
                    let K::Closure { captures, .. } = &mut divergent.kind else {
                        unreachable!()
                    };
                    captures[1].value = Some(Box::new(n::Expr {
                        source,
                        ty: boolean.clone(),
                        kind: K::Call {
                            callee: Box::new(code(bad)),
                            arguments: vec![],
                            parameter_bindings: vec![],
                        },
                    }));
                    let mut rejected = Analysis::new(&package, true);
                    rejected
                        .callables
                        .insert(code_root(alias), vec![BehaviorCode::plain(code(good))]);
                    assert!(!rejected.expression(
                        &divergent,
                        &mut BTreeSet::from([alias]),
                        false,
                        0
                    ));
                    assert!(rejected.result_code.is_none());
                }
            }
        }
    }
    #[test]
    fn later_absent_variant_cannot_prune_earlier_pattern_computation() {
        let package = n::Package::default();
        let mut analysis = Analysis::new(&package, false);
        let variant = n::Pattern::Variant {
            declaration: DeclarationId {
                package: 0,
                local: 1,
            },
            index: 0,
            fields: vec![n::Pattern::Wild],
        };
        let fields = BTreeMap::from([(vec![1, VARIANT_FIELD, 0, 0], vec![])]);
        let structural = n::Pattern::Tuple(vec![n::Pattern::Wild, variant.clone()]);
        assert!(analysis.code_pattern_absent(&structural, &fields, &mut vec![], 0));
        let computed = n::Pattern::Tuple(vec![
            n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(n::Expr {
                    source: n::SourceRange { start: 0, end: 1 },
                    ty: Term::Primitive(P::Bool),
                    kind: K::Literal(n::Literal::Bool(false)),
                }),
                next: Box::new(n::Pattern::Wild),
            },
            variant,
        ]);
        assert!(!analysis.code_pattern_absent(&computed, &fields, &mut vec![], 0));
    }

    #[test]
    fn loop_bottom_cannot_be_exported_as_absent_capture_evidence() {
        let package = n::Package::default();
        let mut analysis = Analysis::new(&package, false);
        let binding = n::LocalId { owner: 9, index: 1 };
        let source = n::SourceRange { start: 0, end: 1 };
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(P::Bool)),
        };
        let closure = n::Expr {
            source,
            ty: Term::Closure {
                environment: vec![signature.clone()],
                signature: Box::new(signature.clone()),
            },
            kind: K::Closure {
                parameters: vec![],
                captures: vec![n::Capture {
                    binding,
                    ty: signature.clone(),
                    value: None,
                }],
                body: std::sync::Arc::new(n::Expr {
                    source,
                    ty: Term::Primitive(P::Bool),
                    kind: K::Call {
                        callee: Box::new(n::Expr {
                            source,
                            ty: signature,
                            kind: K::Local(binding),
                        }),
                        arguments: vec![],
                        parameter_bindings: vec![],
                    },
                }),
            },
        };
        let facts = BTreeMap::from([(code_root(binding), LoopCode::Bottom)]);
        assert_eq!(
            analysis.loop_code_value_at(&closure, &[], &facts, 0),
            LoopCode::Bottom
        );
        assert!(analysis.closures.is_empty());
        assert_eq!(
            analysis.join_loop_codes(LoopCode::Bottom, LoopCode::Known(vec![])),
            LoopCode::Known(vec![])
        );
        assert_eq!(
            analysis.join_loop_codes(LoopCode::Unknown, LoopCode::Known(vec![])),
            LoopCode::Unknown
        );
        assert_ne!(
            LoopCode::Bottom.to_string(),
            LoopCode::Known(vec![]).to_string()
        );
        assert_eq!(
            LoopCode::Known(vec![]).to_string(),
            "(loop-code absent-variant)"
        );
    }

    #[test]
    fn absent_code_leaf_is_not_unknown_or_an_absent_outer_variant() {
        let package = n::Package::default();
        let mut analysis = Analysis::new(&package, false);
        let pattern = n::Pattern::Variant {
            declaration: DeclarationId {
                package: 0,
                local: 1,
            },
            index: 0,
            fields: vec![n::Pattern::Wild],
        };
        let direct = BTreeMap::from([(vec![VARIANT_FIELD, 0, 0], vec![])]);
        assert!(analysis.code_pattern_absent(&pattern, &direct, &mut vec![], 0));
        assert!(!analysis.code_pattern_absent(&pattern, &BTreeMap::new(), &mut vec![], 0));
        let nested = BTreeMap::from([(vec![VARIANT_FIELD, 0, 0, VARIANT_FIELD, 1, 0], vec![])]);
        assert!(!analysis.code_pattern_absent(&pattern, &nested, &mut vec![], 0));
        assert!(
            analysis
                .join_code_fields(direct.clone(), BTreeMap::new())
                .is_empty()
        );
        assert_eq!(
            analysis.join_code_fields(direct.clone(), direct.clone()),
            direct
        );
    }

    #[test]
    fn aggregate_loop_fixed_point_recovers_after_budget_exhaustion() {
        let package = n::Package::default();
        let binding = n::LocalId { owner: 3, index: 1 };
        let source = n::SourceRange { start: 0, end: 1 };
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(P::Bool)),
        };
        let code = |local| n::Expr {
            source,
            ty: signature.clone(),
            kind: K::Function(DeclarationId { package: 0, local }),
        };
        let place = CallablePlace {
            binding,
            fields: vec![0],
        };
        let body = n::Expr {
            source,
            ty: Term::Primitive(P::Void),
            kind: K::Assign {
                place: n::Place {
                    root: binding,
                    fields: vec![],
                },
                value: Box::new(n::Expr {
                    source,
                    ty: Term::Tuple(vec![signature.clone()]),
                    kind: K::Tuple(vec![code(2)]),
                }),
            },
        };
        let mut analysis = Analysis::new(&package, false);
        analysis
            .callables
            .insert(place.clone(), vec![BehaviorCode::plain(code(1))]);
        let before = analysis.callables.clone();
        let mut reached_success = false;
        for budget in 0..512 {
            analysis.callables = before.clone();
            analysis.fuel = budget;
            if analysis.loop_code_closure(&body) {
                reached_success = true;
                break;
            }
            assert_eq!(
                analysis.callables, before,
                "partial publication at budget {budget}"
            );
        }
        assert!(reached_success);
        analysis.callables = before.clone();
        analysis.fuel = 10_000;
        assert!(analysis.loop_code_closure(&body));
        let candidates = &analysis.callables[&place];
        assert_eq!(candidates.len(), 2);
        for expected in [1, 2] {
            assert!(
                candidates
                    .iter()
                    .any(|code| matches!(code.kind,K::Function(id) if id.local==expected))
            );
        }
        let fixed = analysis.callables.clone();
        assert!(analysis.loop_code_closure(&body));
        assert_eq!(analysis.callables, fixed);
    }

    #[test]
    fn snapshot_identity_budget_and_dump_include_captured_keys() {
        let binding = n::LocalId { owner: 8, index: 1 };
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(P::Void)),
        };
        let source = n::SourceRange { start: 0, end: 0 };
        let definition = n::Expr {
            source,
            ty: Term::Closure {
                environment: vec![signature.clone()],
                signature: Box::new(signature.clone()),
            },
            kind: K::Closure {
                parameters: vec![],
                captures: vec![n::Capture {
                    binding,
                    ty: signature,
                    value: None,
                }],
                body: std::sync::Arc::new(n::Expr {
                    source,
                    ty: Term::Primitive(P::Void),
                    kind: K::Unit,
                }),
            },
        };
        let mut first = BehaviorCode::plain(definition);
        first
            .captures
            .insert(code_root(binding), vec![CodeKey::Closure(1)]);
        let mut second = first.clone();
        second
            .captures
            .insert(code_root(binding), vec![CodeKey::Closure(2)]);
        assert_ne!(first, second);
        assert!(!first.charge(&mut 1));
        assert!(first.charge(&mut 1000));
        let dump = first.to_string();
        assert!(dump.starts_with("(behavior-code (definition "), "{dump}");
        assert!(dump.contains("(candidates (closure-key 1))"), "{dump}");
        assert_eq!(
            dump.chars().filter(|c| *c == '(').count(),
            dump.chars().filter(|c| *c == ')').count()
        );
        let mut projected = first.clone();
        let keys = projected.captures.remove(&code_root(binding)).unwrap();
        projected.captures.insert(
            CallablePlace {
                binding,
                fields: vec![0, 1],
            },
            keys,
        );
        assert_ne!(first, projected);
        assert!(projected.to_string().contains("(fields 0 1)"));
        let mut budget = 1;
        assert!(!projected.charge(&mut budget));
        assert!(projected.charge(&mut 1000));
    }
}
type Key = (DeclarationId, Vec<Term>, Inputs);

pub(super) fn validate_runtime_promises(
    package: &n::Package,
    protocols: &Protocols,
) -> Result<(), Error> {
    let mut analysis = Analysis::new(package, false);
    analysis.protocols = protocols.clone();
    for function in &package.functions {
        if function.phase != n::FunctionPhase::Runtime || !function.pure {
            continue;
        }
        let types = function
            .type_parameters
            .iter()
            .map(|declaration| Term::Rigid {
                declaration: *declaration,
                sort: Sort::Type,
            })
            .collect::<Vec<_>>();
        if !analysis.function(function.declaration, &types, 0) {
            return Err(Error {
                source: function.body.source,
                message: format!(
                    "runtime pure function `{}` lacks P4 evidence for its calls, storage or resource behavior",
                    function.name
                ),
                needs_context: false,
                needs_tactic: false,
                needs_capability: false,
                incompatible_context: false,
            });
        }
    }
    Ok(())
}

pub(super) fn validate_defaults(
    package: &n::Package,
    obligations: &[(n::Expr, Span)],
    protocols: &Protocols,
) -> Result<(), Error> {
    let mut analysis = Analysis::new(package, true);
    analysis.protocols = protocols.clone();
    for (value, span) in obligations {
        if !analysis.expression(value, &mut BTreeSet::new(), false, 0) {
            return Err(error(
                *span,
                "default expression requires P3 stability/termination evidence; call graph or operations could not be certified",
            ));
        }
    }
    Ok(())
}

struct Analysis<'a> {
    package: &'a n::Package,
    protocols: Protocols,
    functions: HashMap<DeclarationId, &'a n::Function>,
    tuple_heads: HashSet<DeclarationId>,
    finite: bool,
    resource_scope: bool,
    callables: BTreeMap<CallablePlace, Codes>,
    closures: Codes,
    result_code: Option<Codes>,
    result_fields: BTreeMap<Vec<usize>, Codes>,
    active: HashSet<Key>,
    proven: HashMap<Key, bool>,
    fuel: usize,
}

impl<'a> Analysis<'a> {
    fn new(package: &'a n::Package, finite: bool) -> Self {
        Self {
            package,
            protocols: Protocols::default(),
            functions: package
                .functions
                .iter()
                .map(|f| (f.declaration, f))
                .collect(),
            tuple_heads: package
                .normal_forms
                .iter()
                .filter(|head| head.role == nir::normal_forms::Role::Tuple)
                .map(|head| head.declaration)
                .collect(),
            finite,
            resource_scope: false,
            callables: BTreeMap::new(),
            closures: vec![],
            result_code: None,
            result_fields: BTreeMap::new(),
            active: HashSet::new(),
            proven: HashMap::new(),
            fuel: 100_000,
        }
    }

    fn copy_type(&self, ty: &Term) -> bool {
        fn inspect(ty: &Term, heads: &HashSet<DeclarationId>, depth: usize) -> bool {
            if depth >= 128 {
                return false;
            }
            match ty {
                Term::Tuple(fields) => fields.iter().all(|ty| inspect(ty, heads, depth + 1)),
                Term::Record(fields) => fields.iter().all(|(_, ty)| inspect(ty, heads, depth + 1)),
                Term::NormalForm {
                    head, arguments, ..
                } if heads.contains(head) => {
                    matches!(arguments.as_slice(), [Term::TypeListValue(fields)]
                        if fields.iter().all(|ty| inspect(ty, heads, depth + 1)))
                }
                _ => copy_scalar(ty),
            }
        }
        inspect(ty, &self.tuple_heads, 0)
    }

    fn expression(
        &mut self,
        value: &n::Expr,
        private: &mut BTreeSet<n::LocalId>,
        returns: bool,
        depth: usize,
    ) -> bool {
        self.result_code = None;
        self.result_fields.clear();
        let valid = self.check_expression(value, private, returns, depth);
        if !valid {
            self.result_fields.clear();
        }
        let returned = self.result_code.take();
        self.result_code = if valid {
            match &value.kind {
                K::Function(_) | K::FunctionInstance { .. } => {
                    Some(vec![BehaviorCode::plain(value.clone())])
                }
                K::Closure { .. } => returned,
                K::Local(binding) => self
                    .callables
                    .get(&code_root(*binding))
                    .and_then(|codes| copy_codes(codes, &mut self.fuel)),
                K::Call { .. }
                | K::If { .. }
                | K::Match { .. }
                | K::Block { .. }
                | K::CallPreparation { .. }
                | K::Field { .. }
                | K::AnonymousField { .. } => returned,
                _ => None,
            }
        } else {
            None
        };
        if !matches!(
            value.kind,
            K::Local(_)
                | K::Record { .. }
                | K::Variant { .. }
                | K::Tuple(_)
                | K::AnonymousRecord { .. }
                | K::Call { .. }
                | K::If { .. }
                | K::Match { .. }
                | K::Block { .. }
                | K::CallPreparation { .. }
                | K::Field { .. }
                | K::AnonymousField { .. }
        ) {
            self.result_fields.clear();
        }
        valid
    }

    fn install_code_keys(
        &mut self,
        place: &CallablePlace,
        ty: &Term,
        keys: &[CodeKey],
        source: n::SourceRange,
    ) -> bool {
        let Ok(ty) = crate::verification::memory::owned_path_type(
            Some(self.package),
            ty,
            &place.fields,
            &mut self.fuel,
        ) else {
            return false;
        };
        let Some(codes) = keys
            .iter()
            .map(|key| self.code_value(key, &ty, source))
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        self.callables.insert(place.clone(), codes);
        true
    }

    fn take_code_fields(&mut self) -> BTreeMap<Vec<usize>, Codes> {
        let mut fields = std::mem::take(&mut self.result_fields);
        if let Some(codes) = self.result_code.take() {
            fields.insert(vec![], codes);
        }
        fields
    }

    fn set_code_fields(&mut self, mut fields: BTreeMap<Vec<usize>, Codes>) {
        self.result_code = fields.remove(&vec![]);
        self.result_fields = fields;
    }

    fn join_code_fields(
        &mut self,
        left: BTreeMap<Vec<usize>, Codes>,
        mut right: BTreeMap<Vec<usize>, Codes>,
    ) -> BTreeMap<Vec<usize>, Codes> {
        left.into_iter()
            .filter_map(|(path, codes)| {
                self.fuel = self.fuel.saturating_sub(path.len().saturating_add(1));
                self.join_codes(Some(codes), right.remove(&path))
                    .map(|codes| (path, codes))
            })
            .collect()
    }

    fn aggregate_code_fields(
        &mut self,
        fields: &[n::Expr],
        private: &mut BTreeSet<n::LocalId>,
        returns: bool,
        depth: usize,
    ) -> bool {
        let mut result = BTreeMap::new();
        for (index, field) in fields.iter().enumerate() {
            if !self.expression(field, private, returns, depth + 1) {
                return false;
            }
            for (mut path, codes) in self.take_code_fields() {
                let Some(fuel) = self.fuel.checked_sub(path.len().saturating_add(1)) else {
                    self.fuel = 0;
                    return false;
                };
                self.fuel = fuel;
                path.insert(0, index);
                result.insert(path, codes);
            }
        }
        self.set_code_fields(result);
        true
    }

    fn bind_code_pattern(
        &mut self,
        pattern: &n::Pattern,
        fields: &BTreeMap<Vec<usize>, Codes>,
        path: &mut Vec<usize>,
        private: &mut BTreeSet<n::LocalId>,
        depth: usize,
    ) -> bool {
        if depth >= 128 || self.fuel == 0 {
            return false;
        }
        self.fuel -= 1;
        match pattern {
            n::Pattern::Binding { binding, inner } => {
                private.insert(*binding);
                for (field, codes) in fields {
                    let Some(fuel) = self.fuel.checked_sub(field.len().saturating_add(1)) else {
                        self.fuel = 0;
                        return false;
                    };
                    self.fuel = fuel;
                    if !field.starts_with(path) {
                        continue;
                    }
                    let Some(codes) = copy_codes(codes, &mut self.fuel) else {
                        return false;
                    };
                    self.callables.insert(
                        CallablePlace {
                            binding: *binding,
                            fields: field[path.len()..].to_vec(),
                        },
                        codes,
                    );
                }
                self.bind_code_pattern(inner, fields, path, private, depth + 1)
            }
            n::Pattern::Tuple(patterns)
            | n::Pattern::TypeTuple(patterns)
            | n::Pattern::NormalForm {
                arguments: patterns,
                ..
            }
            | n::Pattern::Record {
                fields: patterns, ..
            }
            | n::Pattern::AnonymousRecord {
                fields: patterns, ..
            } => {
                for (index, pattern) in patterns.iter().enumerate() {
                    path.push(index);
                    let valid = self.bind_code_pattern(pattern, fields, path, private, depth + 1);
                    path.pop();
                    if !valid {
                        return false;
                    }
                }
                true
            }
            n::Pattern::Variant {
                index,
                fields: patterns,
                ..
            } => {
                path.extend([VARIANT_FIELD, *index]);
                for (field, pattern) in patterns.iter().enumerate() {
                    path.push(field);
                    let valid = self.bind_code_pattern(pattern, fields, path, private, depth + 1);
                    path.pop();
                    if !valid {
                        return false;
                    }
                }
                path.truncate(path.len() - 2);
                true
            }
            n::Pattern::Remap { pattern, bindings } => {
                if !self.bind_code_pattern(pattern, fields, path, private, depth + 1) {
                    return false;
                }
                let mut renamed = vec![];
                for (from, to) in bindings {
                    private.remove(from);
                    private.insert(*to);
                    for (place, codes) in &self.callables {
                        let Some(fuel) =
                            self.fuel.checked_sub(place.fields.len().saturating_add(1))
                        else {
                            self.fuel = 0;
                            return false;
                        };
                        self.fuel = fuel;
                        if place.binding != *from {
                            continue;
                        }
                        let Some(codes) = copy_codes(codes, &mut self.fuel) else {
                            return false;
                        };
                        renamed.push((
                            CallablePlace {
                                binding: *to,
                                fields: place.fields.clone(),
                            },
                            codes,
                        ));
                    }
                }
                self.callables
                    .retain(|place, _| !bindings.iter().any(|(from, _)| *from == place.binding));
                self.callables.extend(renamed);
                true
            }
            n::Pattern::Or(patterns) => {
                let visible = self
                    .callables
                    .keys()
                    .map(|place| place.binding)
                    .collect::<BTreeSet<_>>();
                let mut joined: Option<BTreeMap<CallablePlace, Codes>> = None;
                for pattern in patterns {
                    if self.code_pattern_absent(pattern, fields, path, depth + 1) {
                        continue;
                    }
                    // Failed alternatives may have already changed private
                    // locals. Carry those widened facts into later alternatives.
                    if !self.bind_code_pattern(pattern, fields, path, private, depth + 1) {
                        return false;
                    }
                    let mut keys = vec![];
                    for place in self.callables.keys() {
                        let Some(fuel) = self.fuel.checked_sub(place.fields.len() + 1) else {
                            self.fuel = 0;
                            return false;
                        };
                        self.fuel = fuel;
                        if !visible.contains(&place.binding) {
                            keys.push(place.clone());
                        }
                    }
                    let mut branch = keys
                        .into_iter()
                        .map(|key| {
                            let value = self.callables.remove(&key).unwrap();
                            (key, value)
                        })
                        .collect::<BTreeMap<_, _>>();
                    joined = Some(match joined {
                        None => branch,
                        Some(previous) => previous
                            .into_iter()
                            .filter_map(|(place, codes)| {
                                self.join_codes(Some(codes), branch.remove(&place))
                                    .map(|codes| (place, codes))
                            })
                            .collect(),
                    });
                }
                self.callables.extend(joined.unwrap_or_default());
                self.fuel > 0
            }
            n::Pattern::Then { first, value, next } => {
                if !self.bind_code_pattern(first, fields, path, private, depth + 1)
                    || (self.finite && !self.resource_scope && !self.copy_type(&value.ty))
                    || !self.expression(value, private, false, depth + 1)
                {
                    return false;
                }
                let computed = self.take_code_fields();
                self.bind_code_pattern(next, &computed, &mut vec![], private, depth + 1)
            }
            n::Pattern::Guard { pattern, condition } => {
                self.bind_code_pattern(pattern, fields, path, private, depth + 1)
                    && self.copy_type(&condition.ty)
                    && self.expression(condition, private, false, depth + 1)
            }
            _ => true,
        }
    }

    fn code_pattern_absent(
        &mut self,
        pattern: &n::Pattern,
        fields: &BTreeMap<Vec<usize>, Codes>,
        path: &mut Vec<usize>,
        depth: usize,
    ) -> bool {
        if depth >= 128 || self.fuel == 0 {
            self.fuel = 0;
            return false;
        }
        self.fuel -= 1;
        // A later failing subpattern cannot erase computations already executed
        // by an earlier field. Prune only observation-free structural patterns.
        fn structural(pattern: &n::Pattern, fuel: &mut usize, depth: usize) -> bool {
            if depth >= 128 || *fuel == 0 {
                *fuel = 0;
                return false;
            }
            *fuel -= 1;
            match pattern {
                n::Pattern::Then { .. } | n::Pattern::Guard { .. } => false,
                n::Pattern::Binding { inner, .. } | n::Pattern::Remap { pattern: inner, .. } => {
                    structural(inner, fuel, depth + 1)
                }
                n::Pattern::Tuple(fields)
                | n::Pattern::TypeTuple(fields)
                | n::Pattern::Or(fields)
                | n::Pattern::NormalForm {
                    arguments: fields, ..
                }
                | n::Pattern::Record { fields, .. }
                | n::Pattern::AnonymousRecord { fields, .. }
                | n::Pattern::Variant { fields, .. } => fields
                    .iter()
                    .all(|field| structural(field, fuel, depth + 1)),
                _ => true,
            }
        }
        if !structural(pattern, &mut self.fuel, depth) {
            return false;
        }
        match pattern {
            n::Pattern::Binding { inner, .. }
            | n::Pattern::Guard { pattern: inner, .. }
            | n::Pattern::Remap { pattern: inner, .. } => {
                self.code_pattern_absent(inner, fields, path, depth + 1)
            }
            n::Pattern::Variant {
                index,
                fields: patterns,
                ..
            } => {
                path.extend([VARIANT_FIELD, *index]);
                let absent = fields.iter().any(|(field, codes)| {
                    self.fuel = self.fuel.saturating_sub(field.len().saturating_add(1));
                    // A missing nested variant does not make its outer variant
                    // absent. Require a mandatory code leaf of this payload.
                    codes.is_empty()
                        && field.starts_with(path)
                        && !field[path.len()..].contains(&VARIANT_FIELD)
                });
                if absent {
                    path.truncate(path.len() - 2);
                    return true;
                }
                for (index, pattern) in patterns.iter().enumerate() {
                    path.push(index);
                    let absent = self.code_pattern_absent(pattern, fields, path, depth + 1);
                    path.pop();
                    if absent {
                        path.truncate(path.len() - 2);
                        return true;
                    }
                }
                path.truncate(path.len() - 2);
                false
            }
            n::Pattern::Tuple(patterns)
            | n::Pattern::Record {
                fields: patterns, ..
            }
            | n::Pattern::AnonymousRecord {
                fields: patterns, ..
            } => {
                for (index, pattern) in patterns.iter().enumerate() {
                    path.push(index);
                    let absent = self.code_pattern_absent(pattern, fields, path, depth + 1);
                    path.pop();
                    if absent {
                        return true;
                    }
                }
                false
            }
            n::Pattern::Or(patterns) => {
                !patterns.is_empty()
                    && patterns
                        .iter()
                        .all(|pattern| self.code_pattern_absent(pattern, fields, path, depth + 1))
            }
            _ => false,
        }
    }

    fn check_expression(
        &mut self,
        value: &n::Expr,
        private: &mut BTreeSet<n::LocalId>,
        returns: bool,
        depth: usize,
    ) -> bool {
        if depth >= 128 || self.fuel == 0 {
            return false;
        }
        self.fuel -= 1;
        match &value.kind {
            K::Closure { captures, .. } => {
                if !self.supported_captures(captures) {
                    return false;
                }
                let Some(snapshot) =
                    self.initialize_captures(value, captures, private, returns, depth)
                else {
                    return false;
                };
                self.result_code = Some(vec![snapshot]);
                true
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let saved = private.clone();
                let visible_code = self
                    .callables
                    .keys()
                    .map(|place| place.binding)
                    .collect::<BTreeSet<_>>();
                let valid = statements.iter().all(|s| {
                    // A delivered owner may be a block result, never an
                    // implicitly discarded statement in finite defaults.
                    (!self.finite
                        || self.resource_scope
                        || self.copy_type(&s.ty)
                        || is_never(&s.ty))
                        && self.expression(s, private, returns, depth + 1)
                }) && result
                    .as_ref()
                    .is_none_or(|r| self.expression(r, private, returns, depth + 1));
                if result.is_none() {
                    self.result_code = None;
                    self.result_fields.clear();
                }
                *private = saved;
                self.callables
                    .retain(|binding, _| visible_code.contains(&binding.binding));
                valid
            }
            K::Let { binding, value } => {
                if (self.finite && !self.resource_scope && !self.copy_type(&value.ty))
                    || !self.expression(value, private, returns, depth + 1)
                {
                    return false;
                }
                for (fields, code) in self.take_code_fields() {
                    self.callables.insert(
                        CallablePlace {
                            binding: *binding,
                            fields,
                        },
                        code,
                    );
                }
                private.insert(*binding);
                true
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                if otherwise.is_some()
                    || (self.finite && !self.resource_scope && !self.copy_type(&value.ty))
                    || !self.expression(value, private, returns, depth + 1)
                {
                    return false;
                }
                let fields = self.take_code_fields();
                self.bind_code_pattern(pattern, &fields, &mut vec![], private, depth + 1)
                    && guard.as_ref().is_none_or(|guard| {
                        self.copy_type(&guard.ty)
                            && self.expression(guard, private, returns, depth + 1)
                    })
            }
            K::Local(binding) => {
                let mut fields = BTreeMap::new();
                for (place, codes) in &self.callables {
                    if place.binding != *binding || place.fields.is_empty() {
                        continue;
                    }
                    let Some(fuel) = self.fuel.checked_sub(place.fields.len() + 1) else {
                        self.fuel = 0;
                        return false;
                    };
                    self.fuel = fuel;
                    let Some(codes) = copy_codes(codes, &mut self.fuel) else {
                        return false;
                    };
                    fields.insert(place.fields.clone(), codes);
                }
                self.result_fields = fields;
                !self.finite || self.resource_scope || self.copy_type(&value.ty)
            }
            // A default's observation interval is this one finite evaluation.
            // Every transitive call is inspected below; pointer writes, external
            // events, unknown calls and observable cleanup remain inadmissible.
            // Resource validation separately establishes each load's live,
            // initialized target. Never promote this evidence to runtime P4 or
            // cache a computed value across calls/storage versions.
            K::ReadPointer(pointer) if self.finite => {
                self.copy_type(&value.ty)
                    && matches!(&pointer.ty,Term::Pointer(inner) if **inner==value.ty)
                    && self.expression(pointer, private, returns, depth + 1)
            }
            K::PointerField { pointer, .. } | K::PointerAnonymousField { pointer, .. }
                if self.finite =>
            {
                matches!(value.ty, Term::Pointer(_))
                    && matches!(pointer.ty, Term::Pointer(_))
                    && self.expression(pointer, private, returns, depth + 1)
            }
            K::Literal(_) | K::Unit | K::AssociatedValue { .. } => self.copy_type(&value.ty),
            K::Assign { place, value } => {
                if !private.contains(&place.root)
                    || (self.finite && !self.resource_scope && !self.copy_type(&value.ty))
                    || !self.expression(value, private, returns, depth + 1)
                {
                    return false;
                }
                // Widen rather than choosing the last syntactically visited
                // branch. Unknown is sticky, including across loop backedges.
                let path = place
                    .fields
                    .iter()
                    .map(|(_, index)| *index)
                    .collect::<Vec<_>>();
                let mut replacement = self.take_code_fields();
                let affected = self
                    .callables
                    .keys()
                    .filter(|key| key.binding == place.root && key.fields.starts_with(&path))
                    .cloned()
                    .collect::<Vec<_>>();
                for key in affected {
                    let previous = self.callables.remove(&key);
                    let incoming = replacement.remove(&key.fields[path.len()..]);
                    if let Some(codes) = self.join_codes(previous, incoming) {
                        self.callables.insert(key, codes);
                    }
                }
                true
            }
            K::BitAssign {
                place,
                storage_type,
                value,
                ..
            } => {
                private.contains(&place.root)
                    && self.copy_type(storage_type)
                    && self.expression(value, private, returns, depth + 1)
            }
            K::BitUpdate {
                place,
                storage_type,
                selections,
                value,
            } => {
                // Positions are computations too. Inspect every nested window
                // in preparation order before the payload. Bounds and storage
                // validity remain obligations of the program verification pass.
                private.contains(&place.root)
                    && self.copy_type(storage_type)
                    && selections
                        .iter()
                        .flat_map(n::BitSelection::expressions)
                        .all(|input| self.expression(input, private, returns, depth + 1))
                    && self.expression(value, private, returns, depth + 1)
            }
            K::BitWindow {
                value: inner,
                start,
                end,
            } => {
                (self.copy_type(&value.ty) || is_never(&value.ty))
                    && self.expression(inner, private, returns, depth + 1)
                    && self.expression(start, private, returns, depth + 1)
                    && self.expression(end, private, returns, depth + 1)
            }
            K::BitIndex(inner, index) => {
                (self.copy_type(&value.ty) || is_never(&value.ty))
                    && self.expression(inner, private, returns, depth + 1)
                    && self.expression(index, private, returns, depth + 1)
            }
            K::Loop { body, .. } => {
                if self.finite || !self.loop_code_closure(body) {
                    return false;
                }
                self.expression(body, &mut private.clone(), returns, depth + 1)
            }
            K::Break(_) | K::Continue(_) => !self.finite,
            K::Return(value) => {
                returns
                    && value
                        .as_ref()
                        .is_none_or(|value| self.expression(value, private, returns, depth + 1))
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if !self.copy_type(&condition.ty)
                    || !self.expression(condition, private, returns, depth + 1)
                    || !self.expression(then_branch, &mut private.clone(), returns, depth + 1)
                {
                    return false;
                }
                let left = self.take_code_fields();
                let Some(branch) = else_branch else {
                    return true;
                };
                if !self.expression(branch, &mut private.clone(), returns, depth + 1) {
                    return false;
                }
                let right = self.take_code_fields();
                let joined = self.join_code_fields(left, right);
                self.set_code_fields(joined);
                true
            }
            K::Match { value, arms } => {
                if (self.finite && !self.resource_scope && !self.copy_type(&value.ty))
                    || !self.expression(value, private, returns, depth + 1)
                {
                    return false;
                }
                let scrutinee = self.take_code_fields();
                let visible = self
                    .callables
                    .keys()
                    .map(|place| place.binding)
                    .collect::<BTreeSet<_>>();
                let mut codes = None;
                for arm in arms {
                    let absent =
                        self.code_pattern_absent(&arm.pattern, &scrutinee, &mut vec![], depth + 1);
                    if self.fuel == 0 {
                        return false;
                    }
                    if absent {
                        continue;
                    }
                    let mut arm_private = private.clone();
                    if !self.bind_code_pattern(
                        &arm.pattern,
                        &scrutinee,
                        &mut vec![],
                        &mut arm_private,
                        depth + 1,
                    ) || !arm.guard.as_ref().is_none_or(|guard| {
                        self.copy_type(&guard.ty)
                            && self.expression(guard, &mut arm_private, returns, depth + 1)
                    }) || !self.expression(&arm.body, &mut arm_private, returns, depth + 1)
                    {
                        return false;
                    }
                    let result = self.take_code_fields();
                    codes = Some(match codes {
                        None => result,
                        Some(previous) => self.join_code_fields(previous, result),
                    });
                    self.callables
                        .retain(|place, _| visible.contains(&place.binding));
                }
                self.set_code_fields(codes.unwrap_or_default());
                true
            }
            K::Call {
                callee, arguments, ..
            } => {
                // Finite factories may deliver fresh owned values. Their
                // consumers still need explicit transfer/cleanup permission.
                let candidates = if matches!(callee.kind, K::TraitMethod { .. }) {
                    vec![BehaviorCode::plain(callee.as_ref().clone())]
                } else {
                    if !self.expression(callee, private, returns, depth + 1) {
                        return false;
                    }
                    let Some(candidates) = self.result_code.take() else {
                        return false;
                    };
                    candidates
                };
                if candidates.is_empty() {
                    return false;
                }
                let mut inputs = BTreeMap::new();
                // Snapshot identities at each argument's preparation point.
                for arg in arguments {
                    if (self.finite && !self.resource_scope && !self.copy_type(&arg.value.ty))
                        || !self.expression(&arg.value, private, returns, depth + 1)
                    {
                        return false;
                    }
                    for (fields, codes) in self.take_code_fields() {
                        let Some(keys) = codes
                            .iter()
                            .map(|code| self.code_key(code))
                            .collect::<Option<Vec<_>>>()
                        else {
                            return false;
                        };
                        inputs.insert(
                            CallableInput {
                                parameter: arg.parameter,
                                fields,
                            },
                            keys,
                        );
                    }
                }
                let mut returned = None;
                for candidate in candidates {
                    if !self.call_candidate(&candidate, &inputs, value, depth + 1) {
                        return false;
                    }
                    let result = self.take_code_fields();
                    returned = Some(match returned {
                        None => result,
                        Some(previous) => self.join_code_fields(previous, result),
                    });
                }
                self.set_code_fields(returned.unwrap_or_default());
                true
            }
            K::Field {
                value: inner,
                index,
                ..
            }
            | K::AnonymousField {
                value: inner,
                index,
                ..
            } => {
                if self.finite && !self.resource_scope {
                    if !self.copy_type(&value.ty) {
                        return false;
                    }
                    // Observing a Copy field need not copy an affine owner.
                    // Retain the existing direct-place observation allowance;
                    // no code identity is inferred from that owner's type.
                    if !self.copy_type(&inner.ty) {
                        return self.stable_place_read(value);
                    }
                }
                if !self.expression(inner, private, returns, depth + 1) {
                    return false;
                }
                let selected = self
                    .take_code_fields()
                    .into_iter()
                    .filter_map(|(path, codes)| {
                        (path.first() == Some(index)).then(|| (path[1..].to_vec(), codes))
                    })
                    .collect();
                self.set_code_fields(selected);
                true
            }
            // Constructing the delivered nominal value does not retire it.
            // Finite field calls are safe here when prior fields carry no
            // cleanup responsibility. Owned fields still need totality or
            // partial-construction cleanup evidence, not just stable calls.
            K::Record { fields, .. }
                if !self.finite
                    || self.resource_scope
                    || fields.iter().all(|field| self.copy_type(&field.ty)) =>
            {
                self.aggregate_code_fields(fields, private, returns, depth)
            }
            K::Variant { index, fields, .. }
                if !self.finite
                    || self.resource_scope
                    || fields.iter().all(|field| self.copy_type(&field.ty)) =>
            {
                if !self.aggregate_code_fields(fields, private, returns, depth) {
                    return false;
                }
                let selected = self.take_code_fields();
                let Ok(layout) = crate::verification::memory::code_result_bottom(
                    Some(self.package),
                    &value.ty,
                    &mut self.fuel,
                ) else {
                    return false;
                };
                let mut result = layout
                    .into_keys()
                    .filter(|path| !path.starts_with(&[VARIANT_FIELD, *index]))
                    .map(|path| (path, vec![]))
                    .collect::<BTreeMap<_, _>>();
                for (path, codes) in selected {
                    let mut field = vec![VARIANT_FIELD, *index];
                    field.extend(path);
                    result.insert(field, codes);
                }
                self.set_code_fields(result);
                true
            }
            K::Discard(inner) => {
                (!self.finite
                    || self.resource_scope
                    || self.copy_type(&inner.ty)
                    || is_never(&inner.ty))
                    && self.expression(inner, private, returns, depth + 1)
            }
            K::Not(inner)
            | K::FloatNegate(inner)
            | K::BitConvert(inner)
            | K::NewtypeConvert(inner)
            | K::Snapshot(inner)
            | K::TakeBits(inner)
            | K::SymbolName(inner)
            | K::EnumName(inner)
            | K::BitSlice { value: inner, .. } => {
                self.copy_type(&value.ty)
                    && self.copy_type(&inner.ty)
                    && self.expression(inner, private, returns, depth + 1)
            }
            K::And(a, b)
            | K::Or(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b)
            | K::ScalarEqual(a, b)
            | K::BitEqual(a, b)
            | K::BitConcat(a, b)
            | K::FloatBinary {
                left: a, right: b, ..
            } => {
                self.copy_type(&value.ty)
                    && self.expression(a, private, returns, depth + 1)
                    && self.expression(b, private, returns, depth + 1)
            }
            K::IntegerBinary { left, right, .. } => {
                n::machine_integer_type(value).is_some()
                    && self.expression(left, private, returns, depth + 1)
                    && self.expression(right, private, returns, depth + 1)
            }
            K::IntegerNegate(inner) => {
                n::machine_integer_type(value).is_some()
                    && self.expression(inner, private, returns, depth + 1)
            }
            K::Tuple(fields) | K::AnonymousRecord { fields } => {
                (!self.finite || self.resource_scope || self.copy_type(&value.ty))
                    && self.aggregate_code_fields(fields, private, returns, depth)
            }
            // Keep the existing constructor, pattern and code-value checks.
            // In particular, no fallback invokes a closure or invents Copy for
            // nominal values with potentially observable cleanup.
            _ => stable_default(value),
        }
    }

    /// Value transfer is not a state effect. Require the existing affine pass
    /// to account for every path before admitting non-Copy values: leftovers,
    /// replacement, failed preparation and partial construction may run Drop.
    fn transfer_cleanup_is_pure(&mut self, function: &n::Function, inputs: &Inputs) -> bool {
        use crate::verification::moves::{self, Delivery};
        let callable_parameters = inputs
            .keys()
            .filter_map(|slot| {
                let parameter = function.parameters.get(slot.parameter)?;
                let mut definitions = copy_codes(
                    self.callables.get(&CallablePlace {
                        binding: *parameter,
                        fields: slot.fields.clone(),
                    })?,
                    &mut self.fuel,
                )?;
                if definitions.is_empty() {
                    return None;
                }
                // Resource evidence uses a closure representative for mixed sets.
                let closure = definitions
                    .iter()
                    .position(|d| matches!(d.kind, K::Closure { .. }));
                let definition = definitions.remove(closure.unwrap_or(0));
                let (alternatives, choices) = if closure.is_some() {
                    (
                        vec![],
                        definitions
                            .into_iter()
                            .map(|definition| moves::CallableEvidence {
                                definition: definition.definition,
                                alternatives: vec![],
                                choices: vec![],
                                captures: Default::default(),
                                enum_captures: Default::default(),
                            })
                            .collect(),
                    )
                } else {
                    (
                        definitions
                            .into_iter()
                            .map(|code| code.definition)
                            .collect(),
                        vec![],
                    )
                };
                Some(moves::CallableParameter {
                    parameter: slot.parameter,
                    definition: definition.definition,
                    fields: slot.fields.clone(),
                    alternatives,
                    choices,
                    captures: Default::default(),
                    enum_captures: Default::default(),
                })
            })
            .collect::<Vec<_>>();
        let mut copy_types = HashSet::new();
        if self.protocols.drop.is_none() {
            let report = moves::analyze_in_package_with_inputs(
                self.package,
                function,
                |ty| {
                    Ok(if self.copy_type(ty) {
                        Delivery::Copy
                    } else {
                        Delivery::Move
                    })
                },
                self.fuel,
                None,
                &[],
                &callable_parameters,
            );
            let Ok(report) = report else {
                return false;
            };
            self.fuel = self.fuel.saturating_sub(report.entry_steps_used);
            return self.cleanup_report_is_pure(&report, &copy_types, None, 0);
        }
        let mut fuel = self.fuel;
        let context = crate::ptc::Context::from_package(self.package);
        let mut index = crate::implementation::Index::default();
        let budget = crate::ptc::Budget {
            steps: fuel,
            ..Default::default()
        };
        for implementation in &self.package.implementations {
            if implementation.interface.is_none() {
                continue;
            }
            if index
                .insert(
                    crate::implementation::ImplementationHead {
                        declaration: implementation.declaration,
                        interface: implementation.interface,
                        interface_arguments: implementation.interface_arguments.clone(),
                        subject: implementation.subject.clone(),
                        type_parameters: implementation.type_parameters.clone(),
                        requirements: implementation.requirements.clone(),
                    },
                    budget,
                )
                .is_err()
            {
                return false;
            }
        }
        let Ok(context) = context else {
            return false;
        };
        let world = crate::capabilities::World {
            index: &index,
            context: &context,
            traits: &self.package.traits,
            implementations: &self.package.implementations,
            assumptions: &function.capabilities,
        };
        let report = moves::analyze_in_package_with_inputs(
            self.package,
            function,
            |ty| {
                if self.copy_type(ty) || copy_types.contains(ty) {
                    return Ok(Delivery::Copy);
                }
                let (Some(copy), Some(drop)) = (self.protocols.copy, self.protocols.drop) else {
                    return Ok(Delivery::Move);
                };
                use crate::verification::copy::{self, Classification};
                match copy::classify(&world, copy, ty, budget)? {
                    Classification::Intrinsic => {
                        copy_types.insert(ty.clone());
                        Ok(Delivery::Copy)
                    }
                    Classification::Affine => Ok(Delivery::Move),
                    Classification::Pending(reason) => Err(reason),
                    _ => {
                        copy::acquire(self.package, &world, copy, drop, ty, budget, &mut fuel)?;
                        copy_types.insert(ty.clone());
                        Ok(Delivery::Copy)
                    }
                }
            },
            self.fuel,
            None,
            &[],
            &callable_parameters,
        );
        self.fuel = fuel;
        let Ok(report) = report else {
            return false;
        };
        self.fuel = self.fuel.saturating_sub(report.entry_steps_used);
        self.cleanup_report_is_pure(&report, &copy_types, Some(&world), 0)
    }

    fn stable_place_read(&mut self, mut value: &n::Expr) -> bool {
        loop {
            if self.fuel == 0 {
                return false;
            }
            self.fuel -= 1;
            match &value.kind {
                K::Local(_) => return true,
                K::Field { value: inner, .. } | K::AnonymousField { value: inner, .. } => {
                    value = inner
                }
                _ => return false,
            }
        }
    }

    fn trivial_internal_packet(&mut self, ty: &Term, depth: usize) -> bool {
        if depth >= 128 || self.fuel == 0 {
            return false;
        }
        self.fuel -= 1;
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return false;
        };
        if !arguments.is_empty() || !self.package.internal_return_packets.contains(declaration) {
            return false;
        }
        let Some(packet) = self
            .package
            .enums
            .iter()
            .find(|item| item.declaration == *declaration)
        else {
            return false;
        };
        if !packet.type_parameters.is_empty() {
            return false;
        }
        // Only retirement is certified. Ownership delivery still consults the
        // ordinary Copy protocol, and nontrivial payloads still require Drop.
        packet
            .variants
            .iter()
            .flat_map(|variant| &variant.fields)
            .all(|field| self.copy_type(field) || self.trivial_internal_packet(field, depth + 1))
    }

    fn cleanup_report_is_pure(
        &mut self,
        report: &crate::verification::moves::Report,
        copy_types: &HashSet<Term>,
        world: Option<&crate::capabilities::World<'_>>,
        depth: usize,
    ) -> bool {
        use crate::verification::moves::CleanupAction;
        if depth >= 128 || !report.deferred_calls.is_empty() {
            return false;
        }
        for site in &report.cleanup_sites {
            let mut actions: Vec<_> = site.actions.iter().collect();
            while let Some(action) = actions.pop() {
                if self.fuel == 0 {
                    return false;
                }
                self.fuel -= 1;
                // Scalars and acquired Copy representations have no user cleanup.
                // Other responsibilities need a separate Drop behavior contract.
                let ty = match action {
                    CleanupAction::Alternatives(cases) => {
                        if cases.len() > self.fuel {
                            return false;
                        }
                        self.fuel -= cases.len();
                        actions.extend(cases);
                        continue;
                    }
                    CleanupAction::Owned { ty, .. } => ty,
                    CleanupAction::Temporary { temporary, .. } => &temporary.ty,
                    CleanupAction::Prepared { value, .. } => &value.ty,
                    CleanupAction::Planned { .. } => return false,
                };
                if let Term::Nominal { declaration, .. } = ty {
                    let remaining = match action {
                        CleanupAction::Owned { remaining, .. }
                        | CleanupAction::Temporary { remaining, .. } => Some(remaining),
                        _ => None,
                    };
                    if self.package.internal_return_packets.contains(declaration)
                        && remaining
                            .is_some_and(|tree| tree.children.as_ref().is_some_and(Vec::is_empty))
                    {
                        // The payload has already moved out. Only the known
                        // compiler envelope remains; no T destructor is skipped.
                        continue;
                    }
                }
                if site.reason == crate::verification::moves::CleanupReason::Replacement {
                    if let (CleanupAction::Owned { .. }, Term::Nominal { declaration, .. }) =
                        (action, ty)
                    {
                        if self.package.internal_return_packets.contains(declaration)
                            && self
                                .package
                                .internal_return_empty_replacements
                                .get(&(site.source.start, site.source.end))
                                == Some(declaration)
                        {
                            continue;
                        }
                    }
                }
                if !self.copy_type(ty)
                    && !copy_types.contains(ty)
                    && !self.trivial_internal_packet(ty, 0)
                {
                    let (Some(world), Some(protocol)) = (world, self.protocols.drop) else {
                        return false;
                    };
                    let budget = crate::ptc::Budget {
                        steps: self.fuel,
                        ..Default::default()
                    };
                    let Ok(cleanup) = crate::verification::drop::acquire_cleanup(
                        self.package,
                        world,
                        protocol,
                        ty,
                        budget,
                        &mut self.fuel,
                    ) else {
                        return false;
                    };
                    // Every potentially invoked destructor needs body evidence;
                    // this does not weaken Copy's prohibition on duplicating
                    // custom Drop responsibility or the separate access checks.
                    let mut methods = Vec::new();
                    if cleanup
                        .visit_components(&mut self.fuel, &mut |_, resolution, fuel| {
                            use crate::verification::drop::Resolution;
                            match resolution {
                                Resolution::NoCustomDrop(_) => Ok(()),
                                Resolution::Selected(selected) => {
                                    for argument in &selected.method.arguments {
                                        crate::associated::charge_projection_copy(argument, fuel)?;
                                    }
                                    methods.push(selected.method.clone());
                                    Ok(())
                                }
                                _ => Err("cleanup selection is unresolved".into()),
                            }
                        })
                        .is_err()
                    {
                        return false;
                    }
                    if !methods
                        .iter()
                        .all(|method| self.function(method.body, &method.arguments, depth + 1))
                    {
                        return false;
                    }
                }
            }
        }
        report.handlers.iter().all(|handler| {
            self.cleanup_report_is_pure(&handler.invocation, copy_types, world, depth + 1)
        }) && report.defaults.iter().all(|default| {
            self.cleanup_report_is_pure(&default.evaluation, copy_types, world, depth + 1)
        }) && report
            .closures
            .iter()
            .filter(|closure| closure.invoked)
            .all(|closure| {
                self.cleanup_report_is_pure(&closure.invocation, copy_types, world, depth + 1)
            })
    }

    /// Flow-insensitive loop closure covers every program point (including
    /// early exits), not just the final state of one syntactic body traversal.
    fn loop_code_closure(&mut self, body: &n::Expr) -> bool {
        fn pattern_sources<'p>(
            pattern: &'p n::Pattern,
            value: &'p n::Expr,
            path: &mut Vec<usize>,
            out: &mut Vec<(n::LocalId, &'p n::Expr, Vec<usize>)>,
            fuel: &mut usize,
            depth: usize,
        ) -> Result<(), String> {
            if depth >= 128 {
                return Err("loop pattern depth exhausted".into());
            }
            *fuel = fuel
                .checked_sub(path.len().saturating_add(1))
                .ok_or("loop pattern budget exhausted")?;
            match pattern {
                n::Pattern::Binding { binding, inner } => {
                    out.push((*binding, value, path.clone()));
                    pattern_sources(inner, value, path, out, fuel, depth + 1)?;
                }
                n::Pattern::Tuple(fields)
                | n::Pattern::Record { fields, .. }
                | n::Pattern::AnonymousRecord { fields, .. } => {
                    for (index, field) in fields.iter().enumerate() {
                        path.push(index);
                        pattern_sources(field, value, path, out, fuel, depth + 1)?;
                        path.pop();
                    }
                }
                n::Pattern::Variant { index, fields, .. } => {
                    path.extend([VARIANT_FIELD, *index]);
                    for (index, field) in fields.iter().enumerate() {
                        path.push(index);
                        pattern_sources(field, value, path, out, fuel, depth + 1)?;
                        path.pop();
                    }
                    path.truncate(path.len() - 2);
                }
                n::Pattern::Or(patterns) => {
                    for pattern in patterns {
                        pattern_sources(pattern, value, path, out, fuel, depth + 1)?;
                    }
                }
                n::Pattern::Remap { pattern, bindings } => {
                    let start = out.len();
                    pattern_sources(pattern, value, path, out, fuel, depth + 1)?;
                    let end = out.len();
                    for index in start..end {
                        let (binding, value, path) = &out[index];
                        for (from, to) in bindings {
                            *fuel = fuel
                                .checked_sub(path.len().saturating_add(1))
                                .ok_or("loop remap budget exhausted")?;
                            if binding == from {
                                // Candidate computations use the pre-remap ID;
                                // the arm body uses the canonical one. Keep both
                                // equations, rather than renaming the producer.
                                if from != to {
                                    let entry = (*to, *value, path.clone());
                                    out.push(entry);
                                }
                                break;
                            }
                        }
                    }
                }
                n::Pattern::Then {
                    first,
                    value: computed,
                    next,
                } => {
                    pattern_sources(first, value, path, out, fuel, depth + 1)?;
                    pattern_sources(next, computed, &mut vec![], out, fuel, depth + 1)?;
                }
                n::Pattern::Guard { pattern, .. } => {
                    pattern_sources(pattern, value, path, out, fuel, depth + 1)?
                }
                _ => {}
            }
            Ok(())
        }
        let mut raw = Vec::new();
        if nir::visit::expression(body, &mut self.fuel, |node, fuel| {
            match &node.kind {
                K::Let { binding, value } => {
                    raw.push((code_root(*binding), value.as_ref(), vec![], true))
                }
                K::Assign { place, value } => raw.push((
                    CallablePlace {
                        binding: place.root,
                        fields: place.fields.iter().map(|(_, index)| *index).collect(),
                    },
                    value.as_ref(),
                    vec![],
                    false,
                )),
                K::LetPattern { pattern, value, .. } => {
                    let mut bindings = vec![];
                    pattern_sources(pattern, value, &mut vec![], &mut bindings, fuel, 0)?;
                    for (binding, producer, path) in bindings {
                        raw.push((code_root(binding), producer, path, true));
                    }
                }
                K::Match { value, arms } => {
                    for arm in arms {
                        let mut bindings = vec![];
                        pattern_sources(&arm.pattern, value, &mut vec![], &mut bindings, fuel, 0)?;
                        for (binding, producer, path) in bindings {
                            raw.push((code_root(binding), producer, path, true));
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        })
        .is_err()
        {
            return false;
        }
        let mut equations = Vec::new();
        let mut locals = BTreeSet::new();
        for (target, value, prefix, local) in raw {
            let Ok(ty) = crate::verification::memory::owned_path_type(
                Some(self.package),
                &value.ty,
                &prefix,
                &mut self.fuel,
            ) else {
                return false;
            };
            let Ok(leaves) = crate::verification::memory::code_result_bottom(
                Some(self.package),
                &ty,
                &mut self.fuel,
            ) else {
                return false;
            };
            for path in leaves.into_keys() {
                let Some(fuel) = self.fuel.checked_sub(
                    target
                        .fields
                        .len()
                        .saturating_add(path.len())
                        .saturating_add(prefix.len())
                        .saturating_add(1),
                ) else {
                    self.fuel = 0;
                    return false;
                };
                self.fuel = fuel;
                let mut place = target.clone();
                place.fields.extend_from_slice(&path);
                if local {
                    locals.insert(place.clone());
                }
                let mut source = prefix.clone();
                source.extend(path);
                equations.push((place, value, source));
            }
        }
        let Some(mut facts): Option<BTreeMap<_, _>> = self
            .callables
            .iter()
            .map(|(id, codes)| {
                copy_codes(codes, &mut self.fuel).map(|codes| (id.clone(), LoopCode::Known(codes)))
            })
            .collect()
        else {
            return false;
        };
        for local in locals {
            facts.entry(local).or_insert(LoopCode::Bottom);
        }
        for (target, _, _) in &equations {
            facts.entry(target.clone()).or_insert(LoopCode::Unknown);
        }
        loop {
            if self.fuel < facts.len().saturating_add(1) {
                self.fuel = 0;
                return false;
            }
            self.fuel -= facts.len() + 1;
            let mut changed = false;
            for (target, value, path) in &equations {
                let incoming = self.loop_code_value_at(value, path, &facts, 0);
                let Some(previous) = facts
                    .get(target)
                    .and_then(|code| copy_loop_code(code, &mut self.fuel))
                else {
                    return false;
                };
                let Some(copied) = copy_loop_code(&previous, &mut self.fuel) else {
                    return false;
                };
                let next = self.join_loop_codes(copied, incoming);
                if next != previous {
                    facts.insert(target.clone(), next);
                    changed = true;
                }
                if self.fuel == 0 {
                    return false;
                }
            }
            if !changed {
                break;
            }
        }
        // Only entry bindings survive the loop's lexical scope. Local equations
        // were needed to close aliases; their execution still initializes them.
        let mut closed = BTreeMap::new();
        for binding in self.callables.keys() {
            let Some(fuel) = self
                .fuel
                .checked_sub(binding.fields.len().saturating_add(1))
            else {
                self.fuel = 0;
                return false;
            };
            self.fuel = fuel;
            if let Some(LoopCode::Known(incoming)) = facts.get(binding) {
                if incoming.is_empty() && !binding.fields.contains(&VARIANT_FIELD) {
                    continue;
                }
                let Some(incoming) = copy_codes(incoming, &mut self.fuel) else {
                    return false;
                };
                closed.insert(binding.clone(), incoming);
            }
        }
        self.callables = closed;
        true
    }

    fn join_loop_codes(&mut self, left: LoopCode, right: LoopCode) -> LoopCode {
        match (left, right) {
            (LoopCode::Unknown, _) | (_, LoopCode::Unknown) => LoopCode::Unknown,
            (LoopCode::Bottom, value) | (value, LoopCode::Bottom) => value,
            (LoopCode::Known(left), LoopCode::Known(right)) => self
                .join_codes(Some(left), Some(right))
                .map(LoopCode::Known)
                .unwrap_or(LoopCode::Unknown),
        }
    }

    fn loop_code_value_at(
        &mut self,
        value: &n::Expr,
        path: &[usize],
        facts: &BTreeMap<CallablePlace, LoopCode>,
        depth: usize,
    ) -> LoopCode {
        self.try_loop_code_value_at(value, path, facts, depth)
            .unwrap_or(LoopCode::Unknown)
    }

    fn try_loop_code_value_at(
        &mut self,
        value: &n::Expr,
        path: &[usize],
        facts: &BTreeMap<CallablePlace, LoopCode>,
        depth: usize,
    ) -> Option<LoopCode> {
        if depth >= 128 {
            return None;
        }
        self.fuel = self.fuel.checked_sub(path.len().saturating_add(1))?;
        match &value.kind {
            K::Function(_) | K::FunctionInstance { .. } if path.is_empty() => {
                let mut fuel = self.fuel as u64;
                let bounded = constants::charge_copy(value, &mut fuel, Span::default()).is_ok();
                self.fuel = fuel as usize;
                bounded.then(|| LoopCode::Known(vec![BehaviorCode::plain(value.clone())]))
            }
            K::Closure { captures, .. } if path.is_empty() && self.supported_captures(captures) => {
                for capture in captures {
                    let binding = match capture.value.as_deref() {
                        None => capture.binding,
                        Some(n::Expr {
                            kind: K::Local(binding),
                            ..
                        }) => *binding,
                        _ => continue,
                    };
                    for (place, code) in facts {
                        self.fuel = self.fuel.checked_sub(1)?;
                        if place.binding == binding && matches!(code, LoopCode::Bottom) {
                            return Some(LoopCode::Bottom);
                        }
                    }
                }
                self.snapshot_code(value, Some(facts))
                    .map(|code| LoopCode::Known(vec![code]))
            }
            K::Local(binding) => copy_loop_code(
                facts.get(&CallablePlace {
                    binding: *binding,
                    fields: path.to_vec(),
                })?,
                &mut self.fuel,
            ),
            K::Record { fields, .. } | K::Tuple(fields) | K::AnonymousRecord { fields } => {
                let (index, tail) = path.split_first()?;
                Some(self.loop_code_value_at(fields.get(*index)?, tail, facts, depth + 1))
            }
            K::Variant { index, fields, .. } => {
                let [marker, variant, field, tail @ ..] = path else {
                    return None;
                };
                if *marker != VARIANT_FIELD {
                    return None;
                }
                let leaf = crate::verification::memory::owned_path_type(
                    Some(self.package),
                    &value.ty,
                    path,
                    &mut self.fuel,
                )
                .ok()?;
                if !matches!(leaf, Term::Function { .. } | Term::Closure { .. }) {
                    return None;
                }
                if variant != index {
                    return Some(LoopCode::Known(vec![]));
                }
                Some(self.loop_code_value_at(fields.get(*field)?, tail, facts, depth + 1))
            }
            K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => {
                let mut nested = vec![*index];
                nested.extend_from_slice(path);
                Some(self.loop_code_value_at(value, &nested, facts, depth + 1))
            }
            K::If {
                then_branch,
                else_branch: Some(other),
                ..
            } => {
                let left = self.loop_code_value_at(then_branch, path, facts, depth + 1);
                let right = self.loop_code_value_at(other, path, facts, depth + 1);
                Some(self.join_loop_codes(left, right))
            }
            K::Match { arms, .. } => {
                let mut codes = LoopCode::Bottom;
                for arm in arms {
                    let incoming = self.loop_code_value_at(&arm.body, path, facts, depth + 1);
                    codes = self.join_loop_codes(codes, incoming);
                }
                Some(codes)
            }
            K::Call {
                callee, arguments, ..
            } => {
                // Identity inference never authorizes execution: the later
                // behavior walk checks each factory and argument computation.
                let callees = match self.loop_code_value_at(callee, &[], facts, depth + 1) {
                    LoopCode::Bottom => return Some(LoopCode::Bottom),
                    LoopCode::Known(codes) if !codes.is_empty() => codes,
                    _ => return None,
                };
                let mut inputs = Inputs::new();
                for argument in arguments {
                    let leaves = crate::verification::memory::code_result_bottom(
                        Some(self.package),
                        &argument.value.ty,
                        &mut self.fuel,
                    )
                    .ok()?;
                    for field in leaves.into_keys() {
                        match self.loop_code_value_at(&argument.value, &field, facts, depth + 1) {
                            LoopCode::Bottom => return Some(LoopCode::Bottom),
                            LoopCode::Unknown => {}
                            LoopCode::Known(codes) => {
                                if codes.is_empty() && !field.contains(&VARIANT_FIELD) {
                                    return None;
                                }
                                let keys = codes
                                    .iter()
                                    .map(|code| self.code_key(code))
                                    .collect::<Option<Vec<_>>>()?;
                                inputs.insert(
                                    CallableInput {
                                        parameter: argument.parameter,
                                        fields: field,
                                    },
                                    keys,
                                );
                            }
                        }
                    }
                }
                let mut outputs = LoopCode::Bottom;
                for callee in callees {
                    let source = match &callee.kind {
                        K::Function(id) => ReturnedCodeSource::Named(*id, &[]),
                        K::FunctionInstance {
                            declaration,
                            arguments,
                        } => ReturnedCodeSource::Named(*declaration, arguments),
                        K::Closure { .. } => ReturnedCodeSource::Closure(&callee),
                        _ => return None,
                    };
                    let returned = self
                        .returned_code_fields(source, &inputs, value)
                        .and_then(|mut fields| fields.remove(path))
                        .map(LoopCode::Known)
                        .unwrap_or(LoopCode::Unknown);
                    outputs = self.join_loop_codes(outputs, returned);
                }
                Some(outputs)
            }
            K::Block {
                result: Some(result),
                ..
            }
            | K::CallPreparation {
                result: Some(result),
                ..
            } => Some(self.loop_code_value_at(result, path, facts, depth + 1)),
            _ => None,
        }
    }

    fn join_codes(&mut self, left: Option<Codes>, right: Option<Codes>) -> Option<Codes> {
        let (Some(mut left), Some(right)) = (left, right) else {
            return None;
        };
        for code in right {
            if !code.charge(&mut self.fuel) {
                return None;
            }
            for old in &left {
                if !old.charge(&mut self.fuel) {
                    return None;
                }
            }
            let Some(remaining) = self.fuel.checked_sub(left.len().saturating_add(1)) else {
                self.fuel = 0;
                return None;
            };
            self.fuel = remaining;
            if !left.iter().any(|old| {
                old.captures == code.captures
                    && old.ty == code.ty
                    && (old.kind == code.kind
                        || match (&old.kind, &code.kind) {
                            (
                                K::Function(a),
                                K::FunctionInstance {
                                    declaration: b,
                                    arguments,
                                },
                            )
                            | (
                                K::FunctionInstance {
                                    declaration: b,
                                    arguments,
                                },
                                K::Function(a),
                            ) => a == b && arguments.is_empty(),
                            _ => false,
                        })
            }) {
                left.push(code);
            }
        }
        Some(left)
    }

    fn call_candidate(
        &mut self,
        callee: &BehaviorCode,
        inputs: &Inputs,
        result: &n::Expr,
        depth: usize,
    ) -> bool {
        self.result_code = None;
        let (id, types) = match &callee.kind {
            K::Function(id) => (*id, vec![]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.clone()),
            K::Closure { captures, .. } if self.supported_captures(captures) => {
                let valid = self.closure(callee, inputs, depth + 1);
                self.result_code = None;
                self.result_fields.clear();
                if valid {
                    let fields = self
                        .returned_code_fields(ReturnedCodeSource::Closure(callee), inputs, result)
                        .unwrap_or_default();
                    self.set_code_fields(fields);
                }
                return valid;
            }
            K::TraitMethod {
                interface, member, ..
            } => {
                let promised = self
                    .package
                    .traits
                    .iter()
                    .find(|t| t.declaration == *interface)
                    .and_then(|t| t.methods.iter().find(|m| m.declaration == *member))
                    .is_some_and(|m| m.pure && !m.signature.is_comptime_callable());
                if promised && !self.finite {
                    return true;
                }
                let Ok(method) = crate::materialization::resolve_method_instance(
                    self.package,
                    callee,
                    &mut self.fuel,
                ) else {
                    return false;
                };
                (method.body, method.arguments)
            }
            _ => return false,
        };
        let valid = self.function_with_inputs(id, &types, inputs, depth + 1);
        self.result_code = None;
        if valid {
            let fields = self
                .returned_code_fields(ReturnedCodeSource::Named(id, &types), inputs, result)
                .unwrap_or_default();
            self.set_code_fields(fields);
        }
        valid
    }

    /// Recover identities, never a behavior certificate. Every invoking caller
    /// must separately check the factory and its argument computations.
    fn returned_code_fields(
        &mut self,
        source: ReturnedCodeSource<'_>,
        inputs: &Inputs,
        result: &n::Expr,
    ) -> Option<BTreeMap<Vec<usize>, Codes>> {
        use crate::verification::memory::{self, CodeDefinition, CodeResult, PreparedCode};
        if memory::code_result_bottom(Some(self.package), &result.ty, &mut self.fuel)
            .ok()?
            .is_empty()
        {
            return Some(BTreeMap::new());
        }
        let mut definitions = BTreeMap::new();
        for (slot, codes) in inputs {
            let mut values = vec![];
            for key in codes {
                let code = self.code_value(key, &result.ty, result.source)?;
                values.push(self.export_code(&code, 0)?);
            }
            definitions.insert(slot.clone(), values);
        }
        let prepared = definitions
            .iter()
            .map(|(input, values)| {
                let values = values
                    .iter()
                    .map(|value| {
                        Some(match value {
                            PreparedCode::Named(key) => {
                                for ty in &key.arguments {
                                    crate::associated::charge_projection_copy(ty, &mut self.fuel)
                                        .ok()?;
                                }
                                CodeDefinition::Named(key.clone())
                            }
                            PreparedCode::Closure {
                                definition,
                                captures,
                                prepared_captures,
                            } => CodeDefinition::Closure {
                                definition,
                                captures: captures.clone(),
                                prepared_captures: prepared_captures.clone(),
                            },
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some((input.clone(), values))
            })
            .collect::<Option<BTreeMap<_, _>>>()?;
        let code_fields = match source {
            ReturnedCodeSource::Named(id, types) => {
                memory::infer_with_code_inputs(
                    self.package,
                    id,
                    types,
                    &BTreeMap::new(),
                    &prepared,
                    &mut self.fuel,
                )
                .ok()?
                .code_fields
            }
            ReturnedCodeSource::Closure(code) => {
                let PreparedCode::Closure {
                    definition,
                    captures,
                    prepared_captures,
                } = self.export_code(code, 0)?
                else {
                    return None;
                };
                memory::infer_closure_with_prepared_captures(
                    self.package,
                    &definition,
                    &captures,
                    &prepared_captures,
                    &BTreeMap::new(),
                    &prepared,
                    &mut self.fuel,
                )
                .ok()?
                .code_fields
            }
        };
        let mut fields = BTreeMap::new();
        for (path, candidates) in &code_fields {
            let CodeResult::Known(candidates) = candidates else {
                continue;
            };
            if candidates.is_empty() && !path.contains(&VARIANT_FIELD) {
                continue;
            }
            let ty = crate::verification::memory::owned_path_type(
                Some(self.package),
                &result.ty,
                path,
                &mut self.fuel,
            )
            .ok()?;
            let codes = candidates
                .iter()
                .map(|code| self.import_code(code, &ty, result.source, 0))
                .collect::<Option<Vec<_>>>()?;
            fields.insert(path.clone(), codes);
        }
        Some(fields)
    }

    fn export_code(
        &mut self,
        code: &BehaviorCode,
        depth: usize,
    ) -> Option<crate::verification::memory::PreparedCode> {
        use crate::verification::memory::PreparedCode;
        if depth >= 128 || !code.charge(&mut self.fuel) {
            return None;
        }
        match &code.kind {
            K::Function(declaration) => Some(PreparedCode::Named(iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![],
            })),
            K::FunctionInstance {
                declaration,
                arguments,
            } => Some(PreparedCode::Named(iir::InstanceKey {
                declaration: *declaration,
                arguments: arguments.clone(),
            })),
            K::Closure { captures, .. } => {
                let mut prepared = BTreeMap::new();
                for (binding, keys) in &code.captures {
                    if keys.is_empty() && !binding.fields.contains(&VARIANT_FIELD) {
                        continue;
                    }
                    let capture = captures
                        .iter()
                        .find(|capture| capture.binding == binding.binding)?;
                    let ty = crate::verification::memory::owned_path_type(
                        Some(self.package),
                        &capture.ty,
                        &binding.fields,
                        &mut self.fuel,
                    )
                    .ok()?;
                    let mut values = vec![];
                    for key in keys {
                        let value = self.code_value(key, &ty, code.source)?;
                        values.push(self.export_code(&value, depth + 1)?);
                    }
                    prepared.insert(binding.clone(), values);
                }
                Some(PreparedCode::Closure {
                    definition: std::sync::Arc::new(code.definition.clone()),
                    captures: Default::default(),
                    prepared_captures: std::sync::Arc::new(prepared),
                })
            }
            _ => None,
        }
    }

    fn import_code(
        &mut self,
        code: &crate::verification::memory::PreparedCode,
        ty: &Term,
        source: n::SourceRange,
        depth: usize,
    ) -> Option<BehaviorCode> {
        use crate::verification::memory::{CallableResult, PreparedCode};
        if depth >= 128 || self.fuel == 0 {
            return None;
        }
        self.fuel -= 1;
        match code {
            PreparedCode::Named(key) => {
                for ty in &key.arguments {
                    crate::associated::charge_projection_copy(ty, &mut self.fuel).ok()?;
                }
                self.code_value(
                    &CodeKey::Named(key.declaration, key.arguments.clone()),
                    ty,
                    source,
                )
            }
            PreparedCode::Closure {
                definition,
                captures,
                prepared_captures,
            } => {
                let K::Closure {
                    captures: bindings, ..
                } = &definition.kind
                else {
                    return None;
                };
                if !self.supported_captures(bindings) {
                    return None;
                }
                let mut fuel = self.fuel as u64;
                let bounded =
                    constants::charge_copy(definition, &mut fuel, Span::default()).is_ok();
                self.fuel = fuel as usize;
                if !bounded {
                    return None;
                }
                let mut value = BehaviorCode::plain(definition.as_ref().clone());
                for (place, candidates) in captures.iter() {
                    self.fuel = self.fuel.checked_sub(1)?;

                    let binding = bindings
                        .iter()
                        .find(|capture| capture.binding == place.binding)?;
                    let ty = crate::verification::memory::owned_path_type(
                        Some(self.package),
                        &binding.ty,
                        &place.fields,
                        &mut self.fuel,
                    )
                    .ok()?;
                    let keys = match candidates {
                        CallableResult::Known(key) => std::slice::from_ref(key),
                        CallableResult::Candidates(keys) => keys.as_slice(),
                        _ => continue,
                    };
                    let mut captured = vec![];
                    for key in keys {
                        for ty in &key.arguments {
                            crate::associated::charge_projection_copy(ty, &mut self.fuel).ok()?;
                        }
                        let key = CodeKey::Named(key.declaration, key.arguments.clone());
                        let named = self.code_value(&key, &ty, source)?;
                        captured.push(self.code_key(&named)?);
                    }
                    value.captures.insert(place.clone(), captured);
                }
                for (place, candidates) in prepared_captures.iter() {
                    self.fuel = self.fuel.checked_sub(1)?;
                    if value.captures.contains_key(place) {
                        return None;
                    }
                    let binding = bindings
                        .iter()
                        .find(|capture| capture.binding == place.binding)?;
                    let ty = crate::verification::memory::owned_path_type(
                        Some(self.package),
                        &binding.ty,
                        &place.fields,
                        &mut self.fuel,
                    )
                    .ok()?;
                    let mut captured = vec![];
                    for candidate in candidates {
                        let code = self.import_code(candidate, &ty, source, depth + 1)?;
                        captured.push(self.code_key(&code)?);
                    }
                    value.captures.insert(place.clone(), captured);
                }
                Some(value)
            }
        }
    }

    fn supported_captures(&self, captures: &[n::Capture]) -> bool {
        // P4 can transfer affine inputs into an environment. Invocation and
        // retirement are checked by the same move/cleanup reports as ordinary
        // owned parameters; capture types alone do not grant Copy or purity.
        // Finite resource-checked bodies may also capture private owners:
        // their enclosing report checks capture delivery, invocation and even
        // uninvoked environment retirement. Other finite contexts retain the
        // Copy-only admission rule. Neither path grants Copy to the closure.
        !self.finite
            || self.resource_scope
            || captures.iter().all(|capture| self.copy_type(&capture.ty))
    }

    // Capture values are evaluated in source order. Freeze each code value
    // immediately: a later initializer can reassign its original source.
    // Construction checks initializers, while invocation checks the body.
    fn initialize_captures(
        &mut self,
        definition: &n::Expr,
        captures: &[n::Capture],
        private: &mut BTreeSet<n::LocalId>,
        returns: bool,
        depth: usize,
    ) -> Option<BehaviorCode> {
        let mut fuel = self.fuel as u64;
        let bounded = constants::charge_copy(definition, &mut fuel, Span::default()).is_ok();
        self.fuel = fuel as usize;
        if !bounded {
            return None;
        }
        let mut snapshot = BehaviorCode::plain(definition.clone());
        for capture in captures {
            let fields = if let Some(value) = capture.value.as_deref() {
                if !self.expression(value, private, returns, depth + 1) {
                    return None;
                }
                self.take_code_fields()
            } else {
                let mut fields = BTreeMap::new();
                for (place, codes) in &self.callables {
                    if place.binding == capture.binding {
                        self.fuel = self.fuel.checked_sub(place.fields.len() + 1)?;
                        fields.insert(place.fields.clone(), copy_codes(codes, &mut self.fuel)?);
                    }
                }
                fields
            };
            for (fields, codes) in fields {
                let keys = codes
                    .iter()
                    .map(|code| self.code_key(code))
                    .collect::<Option<Vec<_>>>()?;
                snapshot.captures.insert(
                    CallablePlace {
                        binding: capture.binding,
                        fields,
                    },
                    keys,
                );
            }
        }
        Some(snapshot)
    }

    fn snapshot_code(
        &mut self,
        definition: &n::Expr,
        facts: Option<&BTreeMap<CallablePlace, LoopCode>>,
    ) -> Option<BehaviorCode> {
        let mut fuel = self.fuel as u64;
        let bounded = constants::charge_copy(definition, &mut fuel, Span::default()).is_ok();
        self.fuel = fuel as usize;
        if !bounded {
            return None;
        }
        let mut snapshot = BehaviorCode::plain(definition.clone());
        let K::Closure { captures, .. } = &definition.kind else {
            return Some(snapshot);
        };
        for capture in captures {
            let source = match capture.value.as_deref() {
                None => Some(capture.binding),
                Some(n::Expr {
                    kind: K::Local(binding),
                    ..
                }) => Some(*binding),
                _ => None,
            };
            let Some(binding) = source else { continue };
            let mut selected = vec![];
            let mut collect = |place: &CallablePlace, codes: &Codes| -> Option<()> {
                if place.binding != binding {
                    return Some(());
                }
                self.fuel = self.fuel.checked_sub(place.fields.len() + 1)?;
                selected.push((place.fields.clone(), copy_codes(codes, &mut self.fuel)?));
                Some(())
            };
            match facts {
                Some(facts) => {
                    for (place, codes) in facts {
                        if let LoopCode::Known(codes) = codes {
                            collect(place, codes)?;
                        }
                    }
                }
                None => {
                    for (place, codes) in &self.callables {
                        collect(place, codes)?;
                    }
                }
            }
            for (fields, codes) in selected {
                let keys = codes
                    .iter()
                    .map(|code| self.code_key(code))
                    .collect::<Option<Vec<_>>>()?;
                snapshot.captures.insert(
                    CallablePlace {
                        binding: capture.binding,
                        fields,
                    },
                    keys,
                );
            }
        }
        Some(snapshot)
    }

    fn code_key(&mut self, code: &BehaviorCode) -> Option<CodeKey> {
        if !code.charge(&mut self.fuel) {
            return None;
        }
        match &code.kind {
            K::Function(id) => Some(CodeKey::Named(*id, vec![])),
            K::FunctionInstance {
                declaration,
                arguments,
            } => Some(CodeKey::Named(*declaration, arguments.clone())),
            K::Closure { captures, .. } if self.supported_captures(captures) => {
                let initial_fuel = self.fuel;
                let mut fuel = self.fuel as u64;
                let bounded = constants::charge_copy(code, &mut fuel, Span::default()).is_ok();
                self.fuel = fuel as usize;
                if !bounded {
                    return None;
                }
                let comparison_cost = initial_fuel.saturating_sub(self.fuel).max(1);
                for (index, definition) in self.closures.iter().enumerate() {
                    if !definition.charge(&mut self.fuel) {
                        return None;
                    }
                    if self.fuel < comparison_cost {
                        self.fuel = 0;
                        return None;
                    }
                    self.fuel -= comparison_cost;
                    if definition == code {
                        return Some(CodeKey::Closure(index));
                    }
                }
                let index = self.closures.len();
                self.closures.push(code.clone());
                Some(CodeKey::Closure(index))
            }
            _ => None,
        }
    }

    fn code_value(
        &mut self,
        key: &CodeKey,
        ty: &Term,
        source: n::SourceRange,
    ) -> Option<BehaviorCode> {
        if crate::associated::charge_projection_copy(ty, &mut self.fuel).is_err() {
            return None;
        }
        if let CodeKey::Named(_, arguments) = key {
            for ty in arguments {
                if crate::associated::charge_projection_copy(ty, &mut self.fuel).is_err() {
                    return None;
                }
            }
        }
        match key {
            CodeKey::Named(declaration, arguments) => Some(BehaviorCode::plain(n::Expr {
                source,
                ty: ty.clone(),
                kind: K::FunctionInstance {
                    declaration: *declaration,
                    arguments: arguments.clone(),
                },
            })),
            CodeKey::Closure(index) => {
                let code = self.closures.get(*index)?;
                code.charge(&mut self.fuel).then(|| code.clone())
            }
        }
    }

    fn closure(&mut self, definition: &BehaviorCode, inputs: &Inputs, depth: usize) -> bool {
        if depth >= 128 || self.fuel == 0 {
            return false;
        }
        self.fuel -= 1;
        let K::Closure {
            parameters: bindings,
            captures,
            body,
        } = &definition.kind
        else {
            return false;
        };
        if definition.ty.is_comptime_callable() {
            return false;
        }
        let Term::Function { parameters, .. } = definition.ty.callable_signature() else {
            return false;
        };
        if !self.supported_captures(captures)
            || parameters.iter().any(|p| {
                p.sort != Sort::Runtime
                    || (self.finite && !self.resource_scope && !self.copy_type(&p.ty))
            })
        {
            return false;
        }
        let saved = std::mem::take(&mut self.callables);
        for (place, keys) in &definition.captures {
            let Some(capture) = captures
                .iter()
                .find(|capture| capture.binding == place.binding)
            else {
                self.callables = saved;
                return false;
            };
            if !self.install_code_keys(place, &capture.ty, keys, definition.source) {
                self.callables = saved;
                return false;
            }
        }
        for (slot, keys) in inputs {
            let (Some(binding), Some(parameter)) =
                (bindings.get(slot.parameter), parameters.get(slot.parameter))
            else {
                self.callables = saved;
                return false;
            };
            let place = CallablePlace {
                binding: *binding,
                fields: slot.fields.clone(),
            };
            if !self.install_code_keys(&place, &parameter.ty, keys, definition.source) {
                self.callables = saved;
                return false;
            }
        }
        // These are value environments: constructors create owned captures,
        // and formal closure arguments are delivered by value. The enclosing
        // move report must still validate delivery, aliases and every cleanup.
        // This permits updating captured scalar slots, not their pointees.
        let mut private = bindings
            .iter()
            .copied()
            .chain(captures.iter().map(|capture| capture.binding))
            .collect();
        let valid = self.expression(body, &mut private, true, depth + 1);
        self.callables = saved;
        valid
    }

    fn function(&mut self, id: DeclarationId, types: &[Term], depth: usize) -> bool {
        self.function_with_inputs(id, types, &BTreeMap::new(), depth)
    }

    fn function_with_inputs(
        &mut self,
        id: DeclarationId,
        types: &[Term],
        inputs: &Inputs,
        depth: usize,
    ) -> bool {
        if depth >= 128 || self.fuel == 0 {
            return false;
        }
        self.fuel -= 1;
        for ty in types {
            if crate::associated::charge_projection_copy(ty, &mut self.fuel).is_err() {
                return false;
            }
        }
        for input in inputs.keys() {
            let Some(fuel) = self.fuel.checked_sub(input.fields.len().saturating_add(1)) else {
                self.fuel = 0;
                return false;
            };
            self.fuel = fuel;
        }
        for code in inputs.values().flatten() {
            let CodeKey::Named(_, arguments) = code else {
                continue;
            };
            if self.fuel == 0 {
                return false;
            }
            self.fuel -= 1;
            for argument in arguments {
                if crate::associated::charge_projection_copy(argument, &mut self.fuel).is_err() {
                    return false;
                }
            }
        }
        let key = (id, types.to_vec(), inputs.clone());
        // For P4, a backedge assumes only the same behavior obligation, not
        // termination. Every node in this cycle still has its body checked.
        // Failed roots abort validation, so provisional cycle facts never
        // escape into a successfully validated package.
        if self.active.contains(&key) {
            return !self.finite;
        }
        if self.active.iter().any(|(active, _, _)| *active == id) {
            return false;
        }
        if let Some(valid) = self.proven.get(&key) {
            return *valid;
        }
        let Some(function) = self.functions.get(&id).copied() else {
            return false;
        };
        if function.phase != n::FunctionPhase::Runtime
            || function.type_parameters.len() != types.len()
        {
            return false;
        }
        // Charge owned body/type copies before specializing. A long or
        // polymorphically growing call graph cannot bypass the analysis budget.
        let mut fuel = self.fuel as u64;
        let bounded = std::iter::once(&function.body)
            .chain(function.preconditions.iter())
            .chain(function.postconditions.iter())
            .chain(function.defaults.iter().map(|(_, value)| value))
            .all(|value| constants::charge_copy(value, &mut fuel, Span::default()).is_ok());
        self.fuel = fuel as usize;
        if !bounded
            || crate::associated::charge_projection_copy(&function.signature, &mut self.fuel)
                .is_err()
        {
            return false;
        }
        // Instantiation substitutes types only. The source function and its
        // opaque type heads remain untouched, and runtime inputs are not read.
        let Ok(function) = function.instantiate_types(&function.type_parameters, types) else {
            return false;
        };
        let Term::Function { parameters, result } = &function.signature else {
            return false;
        };
        // Rejection after obtaining an owned result could execute its Drop.
        // Without a bound cleanup protocol retain the trivial-only rule;
        // otherwise the mandatory resource report below includes this result's
        // failure-path responsibility and checks its selected Drop behavior.
        if (self.finite
            && self.protocols.drop.is_none()
            && !self.copy_type(result)
            && function
                .postconditions
                .iter()
                .any(|predicate| !matches!(predicate.kind, K::Literal(n::Literal::Bool(true)))))
            || parameters.iter().any(|parameter| {
                parameter.sort != Sort::Runtime
                    || (self.finite
                        && self.protocols.drop.is_none()
                        && !self.copy_type(&parameter.ty))
            })
        {
            return false;
        }
        let saved_callables = std::mem::take(&mut self.callables);
        for (slot, keys) in inputs {
            let (Some(binding), Some(parameter)) = (
                function.parameters.get(slot.parameter),
                parameters.get(slot.parameter),
            ) else {
                self.callables = saved_callables;
                return false;
            };
            let place = CallablePlace {
                binding: *binding,
                fields: slot.fields.clone(),
            };
            if !self.install_code_keys(&place, &parameter.ty, keys, function.body.source) {
                self.callables = saved_callables;
                return false;
            }
        }
        for codes in self.callables.values() {
            for code in codes {
                if !code.charge(&mut self.fuel) {
                    self.callables = saved_callables;
                    return false;
                }
            }
        }
        let entry_callables = self.callables.clone();
        self.active.insert(key.clone());
        let mut private = function.parameters.iter().copied().collect();
        let previous_scope = std::mem::replace(&mut self.resource_scope, false);
        let contracts = function
            .preconditions
            .iter()
            .chain(&function.postconditions)
            .all(|value| {
                self.copy_type(&value.ty) && self.expression(value, &mut private, false, depth + 1)
            });
        // Only complete function bodies have a resource report here. Defaults
        // themselves and untracked captures do not gain ownership permission.
        let checked_resources = self.protocols.drop.is_some();
        self.resource_scope = checked_resources;
        let body = contracts && self.expression(&function.body, &mut private, true, depth + 1);
        self.resource_scope = previous_scope;
        let valid = body
            && ((self.finite && !checked_resources) || {
                self.callables = entry_callables;
                self.transfer_cleanup_is_pure(&function, inputs)
            });
        self.callables = saved_callables;
        self.active.remove(&key);
        self.proven.insert(key, valid);
        valid
    }
}
