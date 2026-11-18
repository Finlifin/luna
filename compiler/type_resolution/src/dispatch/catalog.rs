//! Immutable declaration metadata is owned by the exact source snapshot.
use super::*;
use crate::ptc::DeclarationTheory;
use std::sync::{Arc, OnceLock};

/// An exact immutable source owner. Declaration theory and the implementation
/// directory are prepared independently. Neither can retain a caller's premises.
#[derive(Clone)]
pub struct DeclarationSnapshot(Arc<SnapshotData>);

struct SnapshotData {
    package: Arc<n::Package>,
    theory: OnceLock<DeclarationTheory>,
    metadata: OnceLock<Arc<DeclarationMetadata>>,
}

impl DeclarationSnapshot {
    pub fn new(package: Arc<n::Package>) -> Self {
        Self(Arc::new(SnapshotData {
            package,
            theory: OnceLock::new(),
            metadata: OnceLock::new(),
        }))
    }

    pub fn package(&self) -> &Arc<n::Package> {
        &self.0.package
    }

    /// Fork a closed proof context from this source's validated theory. Only
    /// completed theory is installed, even if several readers prepare it.
    pub fn package_context(
        &self,
        steps: &mut usize,
    ) -> Result<Context, nir::normal_forms::ValidationError> {
        *steps = steps
            .checked_sub(1)
            .ok_or(nir::normal_forms::ValidationError::Budget)?;
        let theory = match self.0.theory.get() {
            Some(theory) => theory,
            None => {
                let completed = DeclarationTheory::from_package_metered(self.package(), steps)?;
                self.0.theory.get_or_init(|| completed)
            }
        };
        Ok(theory.clone().into_context())
    }

    /// Complete the immutable directory before publishing a prepared owner.
    /// Failure leaves the snapshot unprepared and permits a later retry.
    pub fn prepare(&self, budget: &mut Budget) -> Result<(), AdmissionError> {
        self.ensure_metadata(budget).map(|_| ())
    }

    /// Borrow one source-bound implementation view. Premises stay with the
    /// caller and never enter the retained directory.
    pub fn capabilities<'a>(
        &'a self,
        context: &'a Context,
        assumptions: &'a [n::TraitRequirement],
        budget: &mut Budget,
    ) -> Result<crate::capabilities::World<'a>, AdmissionError> {
        self.ensure_metadata(budget)?;
        Ok(crate::capabilities::World {
            index: &self.0.metadata.get().unwrap().index,
            implementations: &self.0.package.implementations,
            traits: &self.0.package.traits,
            assumptions,
            context,
        })
    }

    fn metadata(&self, budget: &mut Budget) -> Result<Arc<DeclarationMetadata>, AdmissionError> {
        if let Some(metadata) = self.0.metadata.get() {
            construction::spend(&mut budget.steps, 1)?;
            return Ok(Arc::clone(metadata));
        }
        Ok(Arc::clone(self.ensure_metadata(budget)?))
    }

    fn ensure_metadata(
        &self,
        budget: &mut Budget,
    ) -> Result<&Arc<DeclarationMetadata>, AdmissionError> {
        if self.0.metadata.get().is_none() {
            let metadata = DeclarationMetadata::build(&self.0.package, budget)?;
            // Only completed metadata is installed. Concurrent constructors, if
            // used, can retain either complete directory of this same source.
            let _ = self.0.metadata.set(metadata);
        }
        Ok(self.0.metadata.get().unwrap())
    }
}

/// The source and its directory cannot be supplied independently. Borrowed
/// scratch sources build privately; retained snapshots share only metadata.
#[derive(Clone, Copy)]
pub enum DeclarationSource<'a> {
    Borrowed(&'a n::Package),
    Snapshot(&'a DeclarationSnapshot),
}
impl<'a> DeclarationSource<'a> {
    pub fn package(self) -> &'a n::Package {
        match self {
            Self::Borrowed(package) => package,
            Self::Snapshot(snapshot) => snapshot.package(),
        }
    }

    pub fn package_context(
        self,
        steps: &mut usize,
    ) -> Result<Context, nir::normal_forms::ValidationError> {
        match self {
            Self::Snapshot(snapshot) => snapshot.package_context(steps),
            Self::Borrowed(package) => Context::from_package_metered(package, steps),
        }
    }

    /// A source VM selects its declaration view when it opens the package.
    /// Keep an existing owner, including its completed theory and directory.
    /// A borrowed source needs a bounded owned copy of the declarations.
    pub fn retain_for_invocation(
        self,
        fuel: &mut usize,
    ) -> Result<DeclarationSnapshot, nir::copy_budget::CopyBudgetExhausted> {
        match self {
            Self::Snapshot(snapshot) => {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or(nir::copy_budget::CopyBudgetExhausted::Steps)?;
                Ok(snapshot.clone())
            }
            Self::Borrowed(_) => self.retain_declarations(fuel),
        }
    }

    /// Retain declarations without importing selected execution dispatch. A
    /// clean snapshot shares its exact owner and completed directory. Borrowed
    /// sources or snapshots with dispatch are copied only after a bounded
    /// preflight; a projected snapshot starts with an unprepared directory.
    /// This is ownership transfer, not phase validation or calling authority.
    pub fn retain_declarations(
        self,
        fuel: &mut usize,
    ) -> Result<DeclarationSnapshot, nir::copy_budget::CopyBudgetExhausted> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or(nir::copy_budget::CopyBudgetExhausted::Steps)?;
        match self {
            Self::Snapshot(snapshot) if snapshot.package().dispatch_instances.is_empty() => {
                Ok(snapshot.clone())
            }
            _ => nir::copy_budget::clone_declaration_source(self.package(), fuel)
                .map(|package| DeclarationSnapshot::new(Arc::new(package))),
        }
    }

    pub fn world(
        self,
        budget: &mut Budget,
        context: Option<(&Context, &[n::TraitRequirement])>,
    ) -> Result<DeclarationWorld<'a>, AdmissionError> {
        DeclarationWorld::from_source_metered(self, budget, context)
    }

    pub(super) fn metadata(
        self,
        budget: &mut Budget,
    ) -> Result<Arc<DeclarationMetadata>, AdmissionError> {
        match self {
            Self::Borrowed(package) => DeclarationMetadata::build(package, budget),
            Self::Snapshot(snapshot) => snapshot.metadata(budget),
        }
    }
}

pub(super) struct DeclarationMetadata {
    pub index: Index,
    pub declaration_parameter_sorts: BTreeMap<Id, nir::Sort>,
    pub callable_functions: HashMap<Id, usize>,
    pub runtime_callables: HashMap<Id, usize>,
    pub normal_heads: HashMap<Id, usize>,
}
impl DeclarationMetadata {
    fn build(package: &n::Package, budget: &mut Budget) -> Result<Arc<Self>, AdmissionError> {
        let index = Index::from_package_metered(package, budget)?;
        let input_fuel = &mut budget.steps;
        // Immutable declaration metadata is indexed once per world. No function
        // invocation result, predicate outcome or mutable state is cached here.
        let mut callable_functions = HashMap::new();
        let mut runtime_callables = HashMap::new();
        let mut normal_heads = HashMap::new();
        for (slot, function) in package.functions.iter().enumerate() {
            *input_fuel = input_fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
            if callable_functions
                .insert(function.declaration, slot)
                .is_some()
            {
                return Err("duplicate callable declaration".into());
            }
        }
        for (slot, identity) in package.runtime_function_identities.iter().enumerate() {
            *input_fuel = input_fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
            if runtime_callables
                .insert(identity.declaration, slot)
                .is_some()
            {
                return Err("duplicate runtime callable identity".into());
            }
        }
        for (slot, head) in package.normal_forms.iter().enumerate() {
            *input_fuel = input_fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
            if normal_heads.insert(head.declaration, slot).is_some() {
                return Err("duplicate normal-form callable identity".into());
            }
        }

        construction::spend(input_fuel, index.parameter_domains().len())?;
        let declaration_parameter_sorts = index
            .parameter_domains()
            .iter()
            .map(|(id, domain)| {
                (
                    *id,
                    if *domain == Term::Primitive(nir::Primitive::Type) {
                        nir::Sort::Type
                    } else {
                        nir::Sort::ComptimeValue
                    },
                )
            })
            .collect();
        construction::spend(&mut budget.steps, 1)?;
        Ok(Arc::new(Self {
            index,
            declaration_parameter_sorts,
            callable_functions,
            runtime_callables,
            normal_heads,
        }))
    }
}

impl std::fmt::Display for DeclarationSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-snapshot (source {}) (theory ",
            self.0.package
        )?;
        match self.0.theory.get() {
            Some(theory) => theory.fmt(f)?,
            None => f.write_str("pending")?,
        }
        f.write_str(") (metadata")?;
        match self.0.metadata.get() {
            Some(metadata) => write!(f, " {}", metadata.index)?,
            None => f.write_str(" pending")?,
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::copy_budget::CopyBudgetExhausted;

    #[test]
    fn retention_shares_the_exact_owner_and_completed_directory() {
        let source = DeclarationSnapshot::new(Arc::new(n::Package::default()));
        source.prepare(&mut Budget::default()).unwrap();
        assert!(source.0.metadata.get().is_some());
        assert!(matches!(
            DeclarationSource::Snapshot(&source).retain_declarations(&mut 0),
            Err(CopyBudgetExhausted::Steps)
        ));
        let weak = Arc::downgrade(&source.0);
        let mut fuel = 1;
        let retained = DeclarationSource::Snapshot(&source)
            .retain_declarations(&mut fuel)
            .unwrap();
        assert_eq!(fuel, 0);
        assert!(Arc::ptr_eq(&source.0, &retained.0));
        drop(source);
        assert!(weak.upgrade().is_some());
        drop(retained);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn theory_publication_is_exact_to_source_and_independent_of_lexical_scopes() {
        let id = Id {
            package: 1,
            local: 7,
        };
        let definition = n::Struct {
            declaration: id,
            name: "Item".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            fields: vec![],
        };
        let package = Arc::new(n::Package {
            structs: vec![definition],
            ..Default::default()
        });
        let snapshot = DeclarationSnapshot::new(package.clone());
        let sibling = snapshot.clone();
        assert!(snapshot.package_context(&mut 1).is_err());
        assert!(sibling.0.theory.get().is_none());
        let mut cold = 100;
        let mut local = sibling.package_context(&mut cold).unwrap();
        assert!(cold < 99);
        assert!(snapshot.0.metadata.get().is_none());
        assert!(local.is_nominal_constructor(id));
        let extra = Id { local: 8, ..id };
        local.admit_nominal_constructor(extra);
        local.assume(crate::ptc::Formula::False);

        assert!(snapshot.package_context(&mut 0).is_err());
        let mut warm = 1;
        let closed = snapshot.package_context(&mut warm).unwrap();
        assert_eq!(warm, 0);
        assert!(closed.scope().is_empty() && closed.assumptions().is_empty());
        assert!(closed.is_nominal_constructor(id));
        assert!(!closed.is_nominal_constructor(extra));
        assert!(!snapshot.to_string().contains("(theory pending)"));
        let fresh = DeclarationSnapshot::new(package);
        assert!(fresh.package_context(&mut 1).is_err());
        assert!(fresh.0.theory.get().is_none());
        let empty = DeclarationSource::Borrowed(&n::Package::default())
            .package_context(&mut 100)
            .unwrap();
        assert!(!empty.is_nominal_constructor(id));
    }

    #[test]
    fn projection_is_bounded_drops_dispatch_and_publishes_no_directory() {
        let id = Id {
            package: 1,
            local: 1,
        };
        let package = n::Package {
            function_parameters: vec![nir::function_parameters::Declaration {
                declaration: id,
                parameters: vec![id],
                domains: vec![Term::Tuple(vec![Term::Bool(true); 2048])],
            }],
            dispatch_instances: vec![n::DispatchInstance {
                interface: id,
                interface_arguments: vec![Term::Tuple(vec![Term::Bool(false); 100_000])],
                subject: Term::Bool(false),
                implementation: id,
                arguments: vec![],
                methods: vec![],
                associated: vec![],
                associated_initializers: vec![],
            }],
            ..Default::default()
        };
        let original = package.clone();
        let source = DeclarationSnapshot::new(Arc::new(package));
        for input in [
            DeclarationSource::Borrowed(source.package()),
            DeclarationSource::Snapshot(&source),
        ] {
            assert!(matches!(
                input.retain_declarations(&mut 128),
                Err(CopyBudgetExhausted::Steps)
            ));
            let mut fuel = 10_000;
            let retained = input.retain_declarations(&mut fuel).unwrap();
            let cost = 10_000 - fuel;
            assert!(cost > 2048);
            assert!(!Arc::ptr_eq(&source.0, &retained.0));
            assert!(retained.0.metadata.get().is_none());
            assert!(retained.package().dispatch_instances.is_empty());
            assert_eq!(
                retained.package().function_parameters,
                original.function_parameters
            );
            assert!(matches!(
                input.retain_declarations(&mut (cost - 1)),
                Err(CopyBudgetExhausted::Steps)
            ));
            let mut exact = cost;
            assert_eq!(
                input.retain_declarations(&mut exact).unwrap().package(),
                retained.package()
            );
            assert_eq!(exact, 0);
        }
        assert_eq!(source.package().as_ref(), &original);
    }
}
