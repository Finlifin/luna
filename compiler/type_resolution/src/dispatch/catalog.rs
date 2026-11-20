//! Immutable declaration metadata is owned by the exact source snapshot.
use super::*;
use crate::ptc::DeclarationTheory;
use std::sync::{Arc, OnceLock};

/// An exact immutable source owner. Declaration theory and the implementation
/// directory are prepared independently. Neither can retain a caller's premises.
#[derive(Clone)]
pub struct DeclarationSnapshot(Arc<SnapshotData>);

/// Identifies the checker that can supply completed data for these declaration
/// slots. Independently created checkers may reuse all the same declaration IDs.
#[derive(Clone, Default)]
pub struct MemberDataSource(Arc<std::collections::BTreeSet<(Id, Id)>>);

impl MemberDataSource {
    pub fn new(slots: std::collections::BTreeSet<(Id, Id)>) -> Self {
        Self(Arc::new(slots))
    }

    pub fn contains(&self, row: Id, member: Id) -> bool {
        self.0.contains(&(row, member))
    }
}

struct SnapshotData {
    data: Option<MemberDataSource>,
    package: Arc<n::Package>,
    programs: Arc<nir::associated_initializers::MemberPrograms>,
    theory: OnceLock<DeclarationTheory>,
    metadata: OnceLock<Arc<DeclarationMetadata>>,
}

/// A closed proof context and the complete directory of its exact source.
pub struct ClosedDeclarations {
    source: DeclarationSnapshot,
    context: Context,
    metadata: Arc<DeclarationMetadata>,
}

impl ClosedDeclarations {
    pub fn source(&self) -> &DeclarationSnapshot {
        &self.source
    }

    pub fn context(&self) -> &Context {
        &self.context
    }

    pub fn capabilities(&self) -> crate::capabilities::World<'_> {
        self.metadata
            .capabilities(self.source.package(), &self.context, &[])
    }

    pub fn function(
        &self,
        declaration: Id,
        fuel: &mut usize,
    ) -> Result<Option<&n::Function>, AdmissionError> {
        construction::spend(fuel, 1)?;
        Ok(self.metadata.function(self.source.package(), declaration))
    }

    /// Start a query cache over the already prepared source and closed premises.
    pub fn world(&self) -> DeclarationWorld<'_> {
        DeclarationWorld {
            normalized_types: HashMap::new(),
            scope: DeclarationScope {
                source: DeclarationSource::Snapshot(&self.source),
                metadata: self.metadata.clone(),
                context: self.context.clone(),
                assumptions: Arc::from([]),
            },
        }
    }
}

impl std::fmt::Display for ClosedDeclarations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(closed-declarations {} {})", self.source, self.context)
    }
}

impl DeclarationSnapshot {
    pub fn new(package: Arc<n::Package>) -> Self {
        Self::from_parts(package, Default::default(), None)
    }

    /// Freeze one source, moving executable member templates out of its proof
    /// rows. The directory and declaration headers cannot come from different
    /// revisions. Completed associated data stays visible to type projections.
    pub fn capture_member_programs(
        package: n::Package,
        fuel: &mut usize,
    ) -> Result<Self, AdmissionError> {
        Self::capture_sources(package, fuel, None)
    }

    pub fn capture_for_data_owner(
        package: n::Package,
        fuel: &mut usize,
        owner: &MemberDataSource,
    ) -> Result<Self, AdmissionError> {
        Self::capture_sources(package, fuel, Some(owner.clone()))
    }

    fn capture_sources(
        mut package: n::Package,
        fuel: &mut usize,
        data: Option<MemberDataSource>,
    ) -> Result<Self, AdmissionError> {
        let mut programs = nir::associated_initializers::MemberPrograms::new();
        construction::spend(fuel, package.implementations.len())?;
        for row in &mut package.implementations {
            if row.associated_initializers.is_empty() {
                continue;
            }
            construction::spend(
                fuel,
                row.associated_initializers
                    .len()
                    .saturating_add(row.associated.len()),
            )?;
            let mut members = std::collections::BTreeMap::new();
            for (member, program) in row.associated_initializers.drain(..) {
                if members.insert(member, program).is_some() {
                    return Err("selected implementation has duplicate associated values".into());
                }
            }
            if row
                .associated
                .iter()
                .any(|(member, _)| members.contains_key(member))
            {
                return Err("selected implementation has duplicate associated values".into());
            }
            if programs.insert(row.declaration, members).is_some() {
                return Err("duplicate implementation declaration".into());
            }
        }
        Ok(Self::from_parts(
            Arc::new(package),
            Arc::new(programs),
            data,
        ))
    }

    fn from_parts(
        package: Arc<n::Package>,
        programs: Arc<nir::associated_initializers::MemberPrograms>,
        data: Option<MemberDataSource>,
    ) -> Self {
        Self(Arc::new(SnapshotData {
            package,
            programs,
            data,
            theory: OnceLock::new(),
            metadata: OnceLock::new(),
        }))
    }

    pub(super) fn accepts_data(&self, owner: &MemberDataSource, row: Id, member: Id) -> bool {
        self.0
            .data
            .as_ref()
            .is_some_and(|source| Arc::ptr_eq(&source.0, &owner.0))
            && owner.contains(row, member)
    }

    pub fn member_programs(&self) -> &nir::associated_initializers::MemberPrograms {
        &self.0.programs
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

    /// Both services use one construction allowance. A failed preparation can
    /// leave a completed independent index, but cannot deliver a closed scope.
    pub fn closed(&self, budget: &mut Budget) -> Result<ClosedDeclarations, AdmissionError> {
        let context = DeclarationSource::Snapshot(self).closed_context(&mut budget.steps)?;
        Ok(ClosedDeclarations {
            metadata: self.metadata(budget)?,
            source: self.clone(),
            context,
        })
    }

    /// Borrow the directory in a caller's lexical proof scope.
    pub fn capabilities<'a>(
        &'a self,
        context: &'a Context,
        assumptions: &'a [n::TraitRequirement],
        budget: &mut Budget,
    ) -> Result<crate::capabilities::World<'a>, AdmissionError> {
        Ok(self
            .ensure_metadata(budget)?
            .capabilities(self.package(), context, assumptions))
    }

    fn metadata(&self, budget: &mut Budget) -> Result<Arc<DeclarationMetadata>, AdmissionError> {
        construction::spend(&mut budget.steps, 1)?;
        Ok(Arc::clone(self.ensure_metadata(budget)?))
    }

    fn ensure_metadata(
        &self,
        budget: &mut Budget,
    ) -> Result<&Arc<DeclarationMetadata>, AdmissionError> {
        if let Some(metadata) = self.0.metadata.get() {
            return Ok(metadata);
        }
        let completed = DeclarationMetadata::build(&self.0.package, budget)?;
        Ok(self.0.metadata.get_or_init(|| completed))
    }
}

/// Reusable declaration directory and theory for an immutable borrowed `Package`.
/// Shares prepared metadata across queries without cloning the `Package`.
pub struct BorrowedCatalog<'a> {
    package: &'a n::Package,
    theory: OnceLock<DeclarationTheory>,
    metadata: OnceLock<Arc<DeclarationMetadata>>,
}

impl<'a> BorrowedCatalog<'a> {
    pub fn new(package: &'a n::Package) -> Self {
        Self {
            package,
            theory: OnceLock::new(),
            metadata: OnceLock::new(),
        }
    }

    pub fn package(&self) -> &'a n::Package {
        self.package
    }

    pub fn package_context(
        &self,
        steps: &mut usize,
    ) -> Result<Context, nir::normal_forms::ValidationError> {
        *steps = steps
            .checked_sub(1)
            .ok_or(nir::normal_forms::ValidationError::Budget)?;
        let theory = match self.theory.get() {
            Some(theory) => theory,
            None => {
                let completed = DeclarationTheory::from_package_metered(self.package, steps)?;
                self.theory.get_or_init(|| completed)
            }
        };
        Ok(theory.clone().into_context())
    }

    pub fn prepare(&self, budget: &mut Budget) -> Result<(), AdmissionError> {
        self.ensure_metadata(budget).map(|_| ())
    }

    fn metadata(&self, budget: &mut Budget) -> Result<Arc<DeclarationMetadata>, AdmissionError> {
        construction::spend(&mut budget.steps, 1)?;
        Ok(Arc::clone(self.ensure_metadata(budget)?))
    }

    fn ensure_metadata(
        &self,
        budget: &mut Budget,
    ) -> Result<&Arc<DeclarationMetadata>, AdmissionError> {
        if let Some(metadata) = self.metadata.get() {
            return Ok(metadata);
        }
        let completed = DeclarationMetadata::build(self.package, budget)?;
        Ok(self.metadata.get_or_init(|| completed))
    }
}

/// The source and its directory cannot be supplied independently. Borrowed
/// scratch sources build privately; prepared catalogs and snapshots share metadata.
#[derive(Clone, Copy)]
pub enum DeclarationSource<'a> {
    Borrowed(&'a n::Package),
    Prepared(&'a BorrowedCatalog<'a>),
    Snapshot(&'a DeclarationSnapshot),
}
impl<'a> DeclarationSource<'a> {
    pub fn phase_inspection(self) -> nir::phases::Inspection<'a> {
        let inspection = nir::phases::Inspection::new(self.package());
        match self {
            Self::Snapshot(snapshot) if !snapshot.member_programs().is_empty() => {
                inspection.with_member_programs(snapshot.0.programs.clone())
            }
            _ => inspection,
        }
    }

    pub fn package(self) -> &'a n::Package {
        match self {
            Self::Borrowed(package) => package,
            Self::Prepared(catalog) => catalog.package(),
            Self::Snapshot(snapshot) => snapshot.package(),
        }
    }

    pub fn package_context(
        self,
        steps: &mut usize,
    ) -> Result<Context, nir::normal_forms::ValidationError> {
        match self {
            Self::Snapshot(snapshot) => snapshot.package_context(steps),
            Self::Prepared(catalog) => catalog.package_context(steps),
            Self::Borrowed(package) => Context::from_package_metered(package, steps),
        }
    }

    pub(super) fn closed_context(self, steps: &mut usize) -> Result<Context, AdmissionError> {
        self.package_context(steps).map_err(|error| {
            use nir::normal_forms::ValidationError as E;
            match error {
                E::Budget | E::Newtype(nir::newtypes::ValidationError::Budget) => {
                    AdmissionError::Budget
                }
                error => AdmissionError::Invalid(format!("dispatch type context: {error}")),
            }
        })
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
            Self::Borrowed(_) | Self::Prepared(_) => self.retain_declarations(fuel),
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
            _ => {
                let package = Arc::new(nir::copy_budget::clone_declaration_source(
                    self.package(),
                    fuel,
                )?);
                Ok(match self {
                    Self::Snapshot(snapshot) => DeclarationSnapshot::from_parts(
                        package,
                        snapshot.0.programs.clone(),
                        snapshot.0.data.clone(),
                    ),
                    Self::Borrowed(_) | Self::Prepared(_) => DeclarationSnapshot::new(package),
                })
            }
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
            Self::Prepared(catalog) => catalog.metadata(budget),
            Self::Snapshot(snapshot) => snapshot.metadata(budget),
        }
    }
}

pub(super) struct DeclarationMetadata {
    pub index: Index,
    pub callable_functions: HashMap<Id, usize>,
    pub runtime_callables: HashMap<Id, usize>,
    pub normal_heads: HashMap<Id, usize>,
}
impl DeclarationMetadata {
    pub(super) fn function<'a>(
        &self,
        package: &'a n::Package,
        declaration: Id,
    ) -> Option<&'a n::Function> {
        self.callable_functions
            .get(&declaration)
            .map(|slot| &package.functions[*slot])
    }

    pub(super) fn capabilities<'a>(
        &'a self,
        package: &'a n::Package,
        context: &'a Context,
        assumptions: &'a [n::TraitRequirement],
    ) -> crate::capabilities::World<'a> {
        crate::capabilities::World {
            index: &self.index,
            implementations: &package.implementations,
            traits: &package.traits,
            assumptions,
            context,
        }
    }

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

        construction::spend(&mut budget.steps, 1)?;
        Ok(Arc::new(Self {
            index,
            callable_functions,
            runtime_callables,
            normal_heads,
        }))
    }
}

impl std::fmt::Display for DeclarationSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(declaration-snapshot (source {})", self.package())?;
        if !self.0.programs.is_empty() {
            f.write_str(" (member-programs")?;
            for (owner, members) in self.0.programs.iter() {
                for (member, program) in members {
                    write!(f, " (member {owner} {member} {program})")?;
                }
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

impl DeclarationSnapshot {
    /// Inspect lazy publication separately from the immutable source dump.
    pub fn preparation(&self) -> impl std::fmt::Display + '_ {
        std::fmt::from_fn(|f| {
            f.write_str("(declaration-preparation (theory ")?;
            match self.0.theory.get() {
                Some(theory) => write!(f, "{theory}")?,
                None => f.write_str("pending")?,
            }
            f.write_str(") (metadata")?;
            match self.0.metadata.get() {
                Some(metadata) => write!(f, " {}", metadata.index)?,
                None => f.write_str(" pending")?,
            }
            f.write_str("))")
        })
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
            packed: false,
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
        assert!(snapshot.0.theory.get().is_some());
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
