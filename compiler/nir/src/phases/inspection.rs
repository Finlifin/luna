//! Share declaration positions across package and detached body inspection.
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
pub struct Inspection<'a> {
    source: Source<'a>,
    declarations: Arc<OnceLock<Declarations>>,
}

/// Completed phase traversal of one immutable package, including defaults and
/// contracts. This is not typing, arithmetic safety or invocation authority.
#[derive(Clone)]
pub struct PhaseCheckedPackage<'a> {
    inspection: Inspection<'a>,
    machine_owners: Arc<std::collections::BTreeSet<crate::DeclarationId>>,
}

impl<'a> PhaseCheckedPackage<'a> {
    pub fn package(&self) -> &Package {
        self.inspection.package()
    }

    pub fn machine_arithmetic_functions(
        &self,
    ) -> &std::collections::BTreeSet<crate::DeclarationId> {
        &self.machine_owners
    }

    pub fn inspection_for(&self, package: &Package) -> Option<Inspection<'a>> {
        std::ptr::eq(self.package(), package).then(|| self.inspection.clone())
    }

    pub fn retain_in(&self, source: Arc<Package>) -> Option<PhaseCheckedPackage<'static>> {
        Some(PhaseCheckedPackage {
            inspection: self.inspection.retain_in(source)?,
            machine_owners: Arc::clone(&self.machine_owners),
        })
    }
}

impl std::fmt::Display for PhaseCheckedPackage<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(phase-checked-package (machine-arithmetic")?;
        for owner in self.machine_owners.iter() {
            write!(f, " {owner}")?;
        }
        write!(f, ") {})", self.inspection)
    }
}

impl<'a> Inspection<'a> {
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
    pub fn retain_in(&self, source: Arc<Package>) -> Option<Inspection<'static>> {
        std::ptr::eq(self.package(), source.as_ref()).then(|| Inspection {
            source: Source::Owned(source),
            declarations: self.declarations.clone(),
        })
    }

    /// Share completed declaration domains only with this exact immutable
    /// source. Directory construction also validates normal-form, newtype and
    /// alias metadata. These facts do not certify any body, invocation premise
    /// or runtime resource permission; callers still check those separately.
    /// No work is initiated here and incomplete preparation exposes no table.
    pub fn admitted_parameter_domains(
        &self,
        package: &Package,
    ) -> Option<Arc<std::collections::BTreeMap<crate::DeclarationId, crate::Term>>> {
        if !std::ptr::eq(self.package(), package) {
            return None;
        }
        self.declarations
            .get()
            .map(|declarations| Arc::clone(&declarations.parameter_domains))
    }

    /// Borrow callable metadata for the same source after directory publication.
    pub fn callables_for<'s>(
        &self,
        package: &'s Package,
    ) -> Option<crate::callable_source::Callables<'s>> {
        if !std::ptr::eq(self.package(), package) {
            return None;
        }
        Some(self.declarations.get()?.callables.bind(package))
    }

    pub fn check(&self, body: &Expr, fuel: &mut usize) -> Result<(), Error> {
        let declarations = self.declarations(body.source, fuel)?;
        PhaseContext::new(self.package(), declarations).check_comptime(body, fuel)
    }

    /// Inspect all declaration bodies, defaults and contracts using the same
    /// allowance as metadata preparation. Completed metadata may be reused by
    /// later detached bodies; a successful package check is not cached here.
    pub fn check_package(
        &self,
        fuel: &mut usize,
    ) -> Result<std::collections::BTreeSet<crate::DeclarationId>, Error> {
        let declarations = self.declarations(super::SourceRange { start: 0, end: 0 }, fuel)?;
        PhaseContext::new(self.package(), declarations).check_package(fuel)
    }

    /// Publish evidence only after the entire package traversal succeeds.
    pub fn check_package_with_evidence(
        &self,
        fuel: &mut usize,
    ) -> Result<PhaseCheckedPackage<'a>, Error> {
        let machine_owners = self.check_package(fuel)?;
        Ok(PhaseCheckedPackage {
            inspection: self.clone(),
            machine_owners: Arc::new(machine_owners),
        })
    }

    fn declarations(
        &self,
        source: super::SourceRange,
        fuel: &mut usize,
    ) -> Result<&Declarations, Error> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| Error::budget(source, "phase declaration access budget exhausted"))?;
        let package = self.package();
        if self.declarations.get().is_none() {
            let declarations = Declarations::new(package, fuel)?;
            // Another reader may have completed the same immutable directory.
            // This reader still pays for its work; no partial result is stored.
            let _ = self.declarations.set(declarations);
        }
        Ok(self.declarations.get().expect("completed phase directory"))
    }
}

impl std::fmt::Display for Inspection<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(phase-inspection (declarations {}) (source {}))",
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
            interface: crate::program::FunctionDeclaration {
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
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                requirements: vec![],
                capabilities: vec![],
            },
            body: Expr {
                kind: K::Local(parameter),
                ..flag()
            },
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
    fn completed_phase_evidence_requires_all_bodies_and_retains_exact_source() {
        let source = source();
        let inspection = Inspection::new(&source);
        assert!(inspection.callables_for(&source).is_none());
        assert!(inspection.check_package_with_evidence(&mut 2).is_err());
        assert!(inspection.callables_for(&source).is_none());
        let checked = inspection
            .check_package_with_evidence(&mut 100_000)
            .unwrap();
        assert!(inspection.callables_for(&source).is_some());
        let copied = Arc::new(source.as_ref().clone());
        assert!(inspection.callables_for(&copied).is_none());
        assert!(checked.inspection_for(&copied).is_none());
        assert!(checked.retain_in(copied).is_none());
        let retained = checked.retain_in(source.clone()).unwrap();
        let weak = Arc::downgrade(&source);
        let dump = retained.to_string();
        assert!(dump.starts_with("(phase-checked-package (machine-arithmetic"));
        assert!(dump.contains("\\\"name\\\"\\nnext"));
        drop(checked);
        drop(inspection);
        drop(source);
        assert!(weak.upgrade().is_some());
        assert!(retained.inspection_for(retained.package()).is_some());
        drop(retained);
        assert!(weak.upgrade().is_none());

        let mut invalid = Package {
            functions: vec![identity()],
            ..Default::default()
        };
        invalid.functions[0].body = Expr {
            kind: K::ComptimeValue(Term::Symbol("wrong domain".into())),
            ..flag()
        };
        let inspection = Inspection::new(&invalid);
        // A complete declaration directory cannot certify invalid bodies.
        inspection.check(&flag(), &mut 100_000).unwrap();
        assert!(inspection.admitted_parameter_domains(&invalid).is_some());
        assert!(inspection.callables_for(&invalid).is_some());
        for _ in 0..2 {
            assert!(matches!(
                inspection.check_package_with_evidence(&mut 100_000),
                Err(Error {
                    kind: super::super::ErrorKind::Invalid,
                    ..
                })
            ));
        }
    }

    #[test]
    fn completed_domains_are_shared_only_with_the_exact_source() {
        let source = source();
        let inspection = Inspection::new(&source);
        assert!(inspection.admitted_parameter_domains(&source).is_none());
        inspection.check_package(&mut 2).unwrap_err();
        assert!(inspection.admitted_parameter_domains(&source).is_none());
        inspection.check_package(&mut 100_000).unwrap();
        let domains = inspection.admitted_parameter_domains(&source).unwrap();
        let retained = inspection.retain_in(source.clone()).unwrap();
        assert!(Arc::ptr_eq(
            &domains,
            &retained.admitted_parameter_domains(&source).unwrap()
        ));
        let copied = source.as_ref().clone();
        assert!(retained.admitted_parameter_domains(&copied).is_none());
        // A completed directory does not admit a malformed detached body.
        let malformed = Expr {
            kind: K::ComptimeValue(Term::Symbol("wrong domain".into())),
            ..flag()
        };
        assert_eq!(
            retained.check(&malformed, &mut 1000).unwrap_err().kind,
            super::super::ErrorKind::Invalid
        );
    }

    #[test]
    fn shared_declaration_preparation_is_metered_and_failures_remain_local() {
        let source = source();
        let inspection = Inspection::new(&source);
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
        let cold = Inspection::new(&source);
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
            let inspection = Inspection::new(&source);
            let pending = inspection.to_string();
            assert!(pending.starts_with("(phase-inspection (declarations pending)"));
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
        let other = Inspection::new(&changed);
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
                .starts_with("(phase-inspection (declarations ready)")
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
        let inspection = Inspection::new(&source);
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
        let other = Inspection::new(&changed);
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
