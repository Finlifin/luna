//! Share declaration positions without retaining per-expression permissions.
use super::{Error, Expr, Package, PhaseContext, declarations::Declarations};
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
enum Source<'a> {
    Borrowed(&'a Package),
    Owned(Arc<Package>),
}

/// Phase inspection in one exact, immutable source world. The directory is
/// prepared lazily under the caller's allowance; failed preparation publishes
/// nothing. Every body receives fresh traversal, storage and unsafe checks.
/// This does not certify package typing, resource safety or arbitrary bodies.
#[derive(Clone)]
pub struct ExpressionInspection<'a> {
    source: Source<'a>,
    declarations: Arc<OnceLock<Declarations>>,
}

impl<'a> ExpressionInspection<'a> {
    pub fn new(package: &'a Package) -> Self {
        Self {
            source: Source::Borrowed(package),
            declarations: Arc::new(OnceLock::new()),
        }
    }

    fn package(&self) -> &Package {
        match &self.source {
            Source::Borrowed(package) => package,
            Source::Owned(package) => package,
        }
    }

    /// Transfer source lifetime responsibility only to its exact existing
    /// owner. Matching declaration IDs or equal package content is insufficient.
    pub fn retain_in(&self, source: Arc<Package>) -> Option<ExpressionInspection<'static>> {
        std::ptr::eq(self.package(), source.as_ref()).then(|| ExpressionInspection {
            source: Source::Owned(source),
            declarations: self.declarations.clone(),
        })
    }

    pub fn check(&self, body: &Expr, fuel: &mut usize) -> Result<(), Error> {
        *fuel = fuel.checked_sub(1).ok_or_else(|| {
            Error::budget(body.source, "phase declaration access budget exhausted")
        })?;
        let package = self.package();
        if self.declarations.get().is_none() {
            let declarations = Declarations::new(package, fuel)?;
            // Another reader may have completed the same immutable directory.
            // This reader still pays for its work; no partial result is stored.
            let _ = self.declarations.set(declarations);
        }
        let declarations = self.declarations.get().expect("completed phase directory");
        PhaseContext::new(package, declarations).check_comptime(body, fuel)
    }
}

impl std::fmt::Display for ExpressionInspection<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(expression-inspection (declarations {}) (source {}))",
            if self.declarations.get().is_some() {
                "ready"
            } else {
                "pending"
            },
            self.package()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DeclarationId, Parameter, Primitive, Sort, Term,
        program::{ExprKind as K, Function, FunctionPhase, Literal, LocalId, SourceRange},
    };

    fn flag() -> Expr {
        Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(Literal::Bool(true)),
        }
    }
    fn identity() -> Function {
        let parameter = LocalId { owner: 1, index: 0 };
        Function {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            name: "quoted \"name\"\nnext".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: flag().ty,
                }],
                result: Box::new(flag().ty),
            },
            parameters: vec![parameter],
            type_parameters: vec![],
            body: Expr {
                kind: K::Local(parameter),
                ..flag()
            },
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            requirements: vec![],
            capabilities: vec![],
        }
    }
    fn call(function: &Function) -> Expr {
        Expr {
            kind: K::Call {
                callee: Box::new(Expr {
                    ty: function.callable_type(),
                    kind: K::Function(function.declaration),
                    ..flag()
                }),
                arguments: vec![crate::program::CallArgument {
                    parameter: 0,
                    is_default: false,
                    passing: Default::default(),
                    value: flag(),
                }],
                parameter_bindings: function.parameters.clone(),
            },
            ..flag()
        }
    }
    fn source() -> Arc<Package> {
        Arc::new(Package {
            functions: vec![identity()],
            normal_forms: vec![crate::normal_forms::Head {
                declaration: DeclarationId {
                    package: 1,
                    local: 2,
                },
                role: Default::default(),
                collector: None,
                parameters: vec![],
                requirements: vec![crate::program::TypePredicate::Bool(true); 1000],
            }],
            ..Default::default()
        })
    }

    #[test]
    fn shared_declaration_preparation_is_metered_and_failures_remain_local() {
        let source = source();
        let inspection = ExpressionInspection::new(&source);
        let mut short = 2;
        assert_eq!(
            inspection.check(&flag(), &mut short).unwrap_err().kind,
            super::super::ErrorKind::Budget
        );
        assert!(inspection.declarations.get().is_none());
        let mut fuel = 100_000;
        inspection.check(&flag(), &mut fuel).unwrap();
        let cold_cost = 100_000 - fuel;
        fuel = 100_000;
        let other = inspection.clone();
        other.check(&flag(), &mut fuel).unwrap();
        let warm_cost = 100_000 - fuel;
        assert!(warm_cost < cold_cost);
        assert!(warm_cost > 1, "body inspection still consumes work");
        for allowance in 0..warm_cost {
            let mut remaining = allowance;
            assert_eq!(
                other.check(&flag(), &mut remaining).unwrap_err().kind,
                super::super::ErrorKind::Budget
            );
            assert!(remaining <= allowance);
        }
        other.check(&flag(), &mut warm_cost.clone()).unwrap();
        // A failed body must neither erase valid metadata nor authorize another body.
        let malformed = Expr {
            kind: K::ComptimeValue(Term::Symbol("wrong domain".into())),
            ..flag()
        };
        assert_eq!(
            other.check(&malformed, &mut 1000).unwrap_err().kind,
            super::super::ErrorKind::Invalid
        );
        assert!(inspection.declarations.get().is_some());
        inspection.check(&flag(), &mut warm_cost.clone()).unwrap();
        let cold = ExpressionInspection::new(&source);
        assert_eq!(
            cold.check(&flag(), &mut warm_cost.clone())
                .unwrap_err()
                .kind,
            super::super::ErrorKind::Budget
        );
        assert!(cold.declarations.get().is_none());
    }

    #[test]
    fn retained_owner_preserves_positions_and_rejects_equal_or_changed_worlds() {
        let source = source();
        let weak = Arc::downgrade(&source);
        let body = call(&source.functions[0]);
        let retained = {
            let inspection = ExpressionInspection::new(&source);
            let pending = inspection.to_string();
            assert!(pending.starts_with("(expression-inspection (declarations pending)"));
            assert!(pending.contains("\\\"name\\\"\\nnext"));
            inspection.check(&body, &mut 100_000).unwrap();
            let copied = Arc::new(source.as_ref().clone());
            assert!(inspection.retain_in(copied).is_none());
            inspection.retain_in(source.clone()).unwrap()
        };
        let mut changed = source.as_ref().clone();
        changed.functions[0].parameters[0].index = 1;
        changed.functions[0].body.kind = K::Local(changed.functions[0].parameters[0]);
        let changed = Arc::new(changed);
        assert!(retained.retain_in(changed.clone()).is_none());
        let other = ExpressionInspection::new(&changed);
        assert_eq!(
            other.check(&body, &mut 100_000).unwrap_err().message,
            "call parameter bindings differ from target declaration"
        );
        drop(source);
        assert!(weak.upgrade().is_some());
        retained.check(&body, &mut 1000).unwrap();
        assert!(
            retained
                .to_string()
                .starts_with("(expression-inspection (declarations ready)")
        );
        drop(retained);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn cached_directories_do_not_cache_lexical_unsafe_or_execution_phase() {
        let mut source = source().as_ref().clone();
        source.functions[0].is_unsafe = true;
        let unsafe_call = call(&source.functions[0]);
        let authorized = Expr {
            kind: K::UnsafeScope(Box::new(unsafe_call.clone())),
            ..flag()
        };
        let inspection = ExpressionInspection::new(&source);
        inspection.check(&authorized, &mut 100_000).unwrap();
        assert_eq!(
            inspection.check(&unsafe_call, &mut 1000).unwrap_err().kind,
            super::super::ErrorKind::Invalid
        );
        inspection.check(&authorized, &mut 1000).unwrap();
        let mut changed = source.clone();
        changed.functions[0].is_unsafe = false;
        changed.functions[0].phase = FunctionPhase::Runtime;
        let runtime_call = call(&changed.functions[0]);
        let other = ExpressionInspection::new(&changed);
        assert_eq!(
            other
                .check(&runtime_call, &mut 100_000)
                .unwrap_err()
                .message,
            "runtime function is unavailable during comptime execution"
        );
        inspection.check(&authorized, &mut 1000).unwrap();
    }
}
